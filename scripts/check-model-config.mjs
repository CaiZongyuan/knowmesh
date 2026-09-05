import { readFile } from 'node:fs/promises';
import { parseEnv } from 'node:util';

const config = parseEnv(await readFile('.env', 'utf8'));
const secrets = Object.entries(config)
  .filter(([key, value]) => /KEY|TOKEN|SECRET|PASSWORD/i.test(key) && value)
  .map(([, value]) => value);

function redact(value) {
  let result = String(value);
  for (const secret of secrets) result = result.replaceAll(secret, '[REDACTED]');
  return result;
}

function required(name) {
  if (!config[name]) throw new Error(`Missing ${name}`);
  return config[name];
}

function endpoint(base, suffix) {
  const url = new URL(base);
  if (url.protocol !== 'https:') throw new Error('Model endpoint must use HTTPS');
  if (url.username || url.password || url.search || url.hash) {
    throw new Error('Model endpoint must not contain credentials, query, or fragment');
  }
  if (suffix && !url.pathname.endsWith(suffix)) {
    url.pathname = `${url.pathname.replace(/\/$/, '')}${suffix}`;
  }
  return url;
}

async function request(url, key, body) {
  const response = await fetch(url, {
    method: 'POST',
    headers: { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(90_000),
  });
  const raw = await response.text();
  let data;
  try {
    data = JSON.parse(raw);
  } catch {
    throw new Error(`HTTP ${response.status}: non-JSON response`);
  }
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}: ${redact(data.error?.message ?? data.message ?? data.code ?? 'Request failed').slice(0, 400)}`);
  }
  return data;
}

async function check(name, run) {
  const started = performance.now();
  try {
    const details = await run();
    console.log(JSON.stringify({ check: name, ok: true, elapsed_ms: Math.round(performance.now() - started), ...details }));
  } catch (error) {
    process.exitCode = 1;
    console.log(JSON.stringify({ check: name, ok: false, elapsed_ms: Math.round(performance.now() - started), error: redact(error.message) }));
  }
}

function completionRequest(extra) {
  return {
    model: required('LLM_MODEL'),
    messages: [{ role: 'user', content: 'Return exactly the JSON object {"ok":true,"value":7}.' }],
    max_tokens: 512,
    temperature: 0,
    thinking: { type: 'disabled' },
    ...extra,
  };
}

async function completion(extra) {
  const data = await request(endpoint(required('LLM_BASE_URL'), '/chat/completions'), required('LLM_KEY'), completionRequest(extra));
  const choice = data.choices?.[0];
  if (choice?.finish_reason !== 'stop') throw new Error(`Unexpected finish reason: ${choice?.finish_reason}`);
  const content = choice.message?.content;
  if (typeof content !== 'string' || content.length === 0) throw new Error('Empty completion');
  const parsed = JSON.parse(content);
  if (parsed.ok !== true || parsed.value !== 7 || Object.keys(parsed).length !== 2) {
    throw new Error('Completion did not match the expected JSON object');
  }
  return { model: data.model, usage: data.usage, valid_json: true };
}

await check('llm_json', () => completion({ response_format: { type: 'json_object' } }));
await check('llm_json_schema', () => completion({
  response_format: {
    type: 'json_schema',
    json_schema: {
      name: 'configuration_check', strict: true,
      schema: {
        type: 'object', additionalProperties: false,
        properties: { ok: { type: 'boolean', const: true }, value: { type: 'integer', const: 7 } },
        required: ['ok', 'value'],
      },
    },
  },
}));

await check('embedding_batch', async () => {
  const data = await request(endpoint(required('EMBEDDING_BASE_URL')), required('EMBEDDING_KEY'), {
    model: required('EMBEDDING_MODEL'),
    input: ['Single-cell gene expression and perturbation prediction.', '\u5355\u7ec6\u80de\u57fa\u56e0\u8868\u8fbe\u4e0e\u6270\u52a8\u9884\u6d4b\u3002', 'Install a kitchen sink and replace the plumbing.'],
    encoding_format: 'float',
  });
  if (!Array.isArray(data.data) || data.data.length !== 3) throw new Error('Expected three embeddings');
  const ordered = [...data.data].sort((a, b) => a.index - b.index);
  const dimensions = ordered[0].embedding?.length;
  if (!dimensions) throw new Error('Empty embedding');
  for (const [index, entry] of ordered.entries()) {
    if (entry.index !== index || !Array.isArray(entry.embedding) || entry.embedding.length !== dimensions || !entry.embedding.every(Number.isFinite)) {
      throw new Error('Invalid batch indices, dimensions, or embedding values');
    }
    if (!entry.embedding.some(value => value !== 0)) throw new Error('Zero embedding');
  }
  const cosine = (a, b) => a.reduce((sum, value, index) => sum + value * b[index], 0)
    / Math.sqrt(a.reduce((sum, value) => sum + value ** 2, 0) * b.reduce((sum, value) => sum + value ** 2, 0));
  const related = cosine(ordered[0].embedding, ordered[1].embedding);
  const unrelated = cosine(ordered[0].embedding, ordered[2].embedding);
  if (related <= unrelated) throw new Error('Multilingual similarity smoke check failed');
  return { model: data.model ?? config.EMBEDDING_MODEL, dimensions, batch_size: 3, related_similarity: related, unrelated_similarity: unrelated, usage: data.usage };
});
