import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {createContext, runInContext} from 'node:vm';

const context = createContext({});
for (const file of ['vendor/markdown-it.min.js', 'vendor/highlight.min.js', 'markdown.js']) {
  runInContext(await readFile(`web/${file}`, 'utf8'), context);
}
const app = await readFile('web/app.js', 'utf8');
runInContext(app.slice(app.indexOf('const escape ='), app.indexOf('let data,')), context);
const render = (text, role='assistant') => context.messageBody({role, text});
const markdown = '# Result\n\n**Bold** and *emphasis* with `code`.\n\n- One\n  - Nested\n\n> Quote\n\n| A | B |\n| - | - |\n| 1 | 2 |\n\n```js\nconst value = "hello";\n```';
for (const tag of ['h1','strong','em','code','ul','blockquote','table']) assert.match(render(markdown), new RegExp(`<${tag}[ >]`));
assert.match(render(markdown), /hljs-keyword/);
assert.match(render(markdown, 'reviewer'), /<h1>Result<\/h1>/);
assert.match(render('First line\nSecond line'), /First line<br>\nSecond line/);
assert.match(render('```unknown\n<a>\n```'), /&lt;a&gt;/);
assert.match(render('```json\n{"ok": true}\n```'), /hljs-attr/);
assert.match(render('```js\nconst pending = 1;'), /hljs-keyword/);
assert.match(render('[Docs](https://example.com)'), /target="_blank" rel="noopener noreferrer"/);
for (const text of ['<script>alert(1)</script>', '<img src=x onerror=alert(1)>', '[bad](javascript:alert%281%29)', '[bad](data:text/html,test)']) {
  assert.doesNotMatch(render(text), /<script|<img|href="(?:javascript|data):/);
}
assert.equal(render('**literal**\n<script>', 'user'), '<div class="message-body">**literal**\n&lt;script&gt;</div>');
assert(app.includes('${messageBody(m)}'));
console.log('Markdown rendering, code highlighting, and unsafe content checks passed');
