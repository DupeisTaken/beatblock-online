import { createReadStream, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { extname, join, normalize, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '../web/dist');
const types = {
  '.css': 'text/css',
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.map': 'application/json',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
};
createServer((request, response) => {
  const pathname = decodeURIComponent(new URL(request.url ?? '/', 'http://127.0.0.1').pathname);
  const candidate = normalize(join(root, pathname));
  const file =
    candidate.startsWith(root) && existsSync(candidate) && statSync(candidate).isFile()
      ? candidate
      : join(root, 'index.html');
  response.setHeader('content-type', types[extname(file)] ?? 'application/octet-stream');
  createReadStream(file).pipe(response);
}).listen(4173, '127.0.0.1', () => console.log('Serving web/dist on http://127.0.0.1:4173'));
