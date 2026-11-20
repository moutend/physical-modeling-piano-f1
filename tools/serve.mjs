// Dependency-free static server for the web app: node tools/serve.mjs [port]
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..', 'web');
const port = Number(process.argv[2] ?? process.env.PORT ?? 8080);
const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.wasm': 'application/wasm',
};

const server = createServer(async (req, res) => {
  let path;
  try {
    path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname));
  } catch {
    res.writeHead(400).end('bad request');
    return;
  }
  const file = join(root, path === '/' || path === '\\' ? 'index.html' : path);
  if (!file.startsWith(root)) {
    res.writeHead(403).end();
    return;
  }
  try {
    const body = await readFile(file);
    res.writeHead(200, {
      'Content-Type': types[extname(file)] ?? 'application/octet-stream',
      'Cache-Control': 'no-store',
    });
    res.end(body);
  } catch {
    res.writeHead(404).end('not found');
  }
});

server.on('error', (err) => {
  if (err.code === 'EADDRINUSE') {
    console.error(`ポート ${port} は別のプロセスが使用中です。`);
    console.error(`  使用中のプロセスを調べる: lsof -nP -iTCP:${port} -sTCP:LISTEN`);
    console.error(`  別のポートで起動する:     node tools/serve.mjs ${port + 1}`);
  } else {
    console.error(`サーバを起動できません: ${err.message}`);
  }
  process.exit(1);
});

// Ctrl-C / kill: stop accepting, drop open (keep-alive) connections, exit.
for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.on(sig, () => {
    server.close(() => process.exit(0));
    server.closeAllConnections();
    setTimeout(() => process.exit(0), 500).unref();
  });
}

server.listen(port, '127.0.0.1', () => console.log(`http://localhost:${port} （終了: Ctrl-C）`));
