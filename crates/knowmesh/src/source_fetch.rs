use std::{
    error::Error,
    io::Read,
    sync::Arc,
    time::{Duration, Instant},
};

use knowmesh_core::{
    application::source_fetch::{FetchRequest, too_large, validate_address, validate_url},
    canonical::source::ImportedContent,
    error::{AppError, AppResult, ErrorType},
    ports::SourceFetcher,
};
use reqwest::{
    blocking::Client,
    dns::{Addrs, Name, Resolve, Resolving},
    header,
    redirect::Policy,
};

pub struct HttpSourceFetcher;

struct GuardedResolver {
    allow_private_network: bool,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allow = self.allow_private_network;
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            for address in &addresses {
                validate_address(address.ip(), allow)?;
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

impl SourceFetcher for HttpSourceFetcher {
    fn fetch(&self, request: &FetchRequest) -> AppResult<ImportedContent> {
        let start = Instant::now();
        let deadline = Duration::from_secs(request.fetch_timeout_seconds);
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(request.connect_timeout_seconds))
            .timeout(deadline)
            .dns_resolver(Arc::new(GuardedResolver {
                allow_private_network: request.allow_private_network,
            }))
            .user_agent(concat!("KnowMesh/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| fetch_error(&error))?;
        let mut url = validate_url(&request.url, request.allow_private_network)?;
        for redirects in 0..=5 {
            let remaining = deadline
                .checked_sub(start.elapsed())
                .filter(|duration| !duration.is_zero())
                .ok_or_else(timeout)?;
            let response = client
                .get(url.clone())
                .timeout(remaining)
                .send()
                .map_err(|error| fetch_error(&error))?;
            if [301, 302, 303, 307, 308].contains(&response.status().as_u16()) {
                if redirects == 5 {
                    return Err(AppError::new(
                        ErrorType::Network,
                        "FETCH_REDIRECT_LIMIT",
                        "The source exceeded five redirects.",
                    ));
                }
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(invalid_redirect)?;
                let next = url.join(location).map_err(|_| invalid_redirect())?;
                url = validate_url(next.as_str(), request.allow_private_network)?;
                continue;
            }
            if response.status().as_u16() != 200 {
                return Err(AppError::new(
                    ErrorType::Network,
                    "FETCH_HTTP_STATUS",
                    "The source server did not return a complete successful response.",
                )
                .with_details(serde_json::json!({"status": response.status().as_u16()})));
            }
            if response
                .content_length()
                .is_some_and(|length| length > request.max_bytes)
            {
                return Err(too_large());
            }
            if response
                .headers()
                .get(header::CONTENT_ENCODING)
                .is_some_and(|value| value != "identity")
            {
                return Err(AppError::new(
                    ErrorType::Validation,
                    "FETCH_ENCODING_UNSUPPORTED",
                    "The source server returned an unsupported content encoding.",
                ));
            }
            let mime: mime::Mime = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok())
                .ok_or_else(unsupported_type)?;
            let mime_type = mime.essence_str().to_owned();
            if ![
                "text/plain",
                "text/markdown",
                "text/html",
                "application/pdf",
            ]
            .contains(&mime_type.as_str())
            {
                return Err(unsupported_type());
            }
            let mut bytes = Vec::new();
            response
                .take(request.max_bytes.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|error| fetch_error(&error))?;
            if bytes.len() as u64 > request.max_bytes {
                return Err(too_large());
            }
            return Ok(ImportedContent {
                bytes,
                mime_type,
                final_url: url.into(),
            });
        }
        unreachable!("the redirect limit returns before leaving the loop")
    }
}

fn fetch_error(error: &(dyn Error + 'static)) -> AppError {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(app) = error.downcast_ref::<AppError>() {
            return app.clone();
        }
        if error
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.is_timeout())
            || error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut)
        {
            return timeout();
        }
        current = error.source();
    }
    AppError::new(
        ErrorType::Network,
        "FETCH_FAILED",
        "The source request could not be completed.",
    )
    .retryable(true)
}

fn timeout() -> AppError {
    AppError::new(
        ErrorType::Network,
        "FETCH_TIMEOUT",
        "The source request exceeded its timeout.",
    )
    .retryable(true)
}

fn invalid_redirect() -> AppError {
    AppError::new(
        ErrorType::Network,
        "FETCH_REDIRECT_INVALID",
        "The source server returned an invalid redirect.",
    )
}

fn unsupported_type() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "SOURCE_TYPE_UNSUPPORTED",
        "The source response must declare Markdown, TXT, HTML, or PDF content.",
    )
}
