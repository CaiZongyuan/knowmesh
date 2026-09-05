const token = process.env.NODE_AUTH_TOKEN;
if (!token) throw new Error('NODE_AUTH_TOKEN is missing');

async function get(path) {
  const response = await fetch(new URL(path, 'https://registry.npmjs.org'), {
    headers: { Authorization: `Bearer ${token}` },
    signal: AbortSignal.timeout(30_000),
  });
  return { status: response.status, ok: response.ok, data: await response.json() };
}

const identity = await get('/-/whoami');
if (!identity.ok || typeof identity.data.username !== 'string') {
  console.log(JSON.stringify({ check: 'npm_identity', ok: false, status: identity.status }));
  process.exit(1);
}
const username = identity.data.username;
console.log(JSON.stringify({ check: 'npm_identity', ok: true, username }));

// npm access tries the organization route first and only falls back on 404.
const packages = await get(`/-/user/${encodeURIComponent(username)}/package`);
if (!packages.ok) {
  console.log(JSON.stringify({ check: 'npm_package_permissions', ok: false, status: packages.status }));
  process.exitCode = 1;
} else {
  const permissions = Object.values(packages.data);
  console.log(JSON.stringify({
    check: 'npm_package_permissions', ok: true,
    visible_packages: permissions.length,
    writable_packages: permissions.filter(value => value === 'write' || value === 'read-write').length,
    candidates: ['knowmesh'].map(name => ({ name, permission: packages.data[name] ?? 'not-listed' })),
    publish_verified: false,
  }));
}
