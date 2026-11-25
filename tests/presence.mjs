import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {runInNewContext} from 'node:vm';

const reports = [], timers = [], windowEvents = {}, documentEvents = {};
let focused = true;
const document = {
  visibilityState: 'visible',
  hasFocus: () => focused,
  addEventListener: (event, handler) => { documentEvents[event] = handler; },
};
const source = await readFile('web/app.js', 'utf8');
runInNewContext(source.slice(0, source.indexOf('const states =')), {
  document,
  window: {addEventListener: (event, handler) => { windowEvents[event] = handler; }},
  crypto: {randomUUID: () => 'test-tab'},
  fetch: (url, options) => {
    assert.equal(url, '/api/presence');
    reports.push(JSON.parse(options.body));
    return Promise.resolve();
  },
  setInterval: (handler, interval) => { timers.push({handler, interval}); },
});

assert.equal(reports.at(-1).focused, true, 'Report focus before the board loads');
assert.equal(timers.length, 1);
assert.equal(timers[0].interval, 1200);
timers[0].handler();
assert.equal(reports.at(-1).focused, true, 'Keep focus alive independently of board requests');
focused = false;
windowEvents.blur();
assert.equal(reports.at(-1).focused, false);
focused = true;
windowEvents.focus();
assert.equal(reports.at(-1).focused, true);
document.visibilityState = 'hidden';
documentEvents.visibilitychange();
assert.equal(reports.at(-1).focused, false, 'Hidden tabs are unfocused');
assert.deepEqual(reports.map(r => r.sequence), [1, 2, 3, 4, 5]);
assert(reports.every(r => r.id === 'test-tab' && r.rendered_at === 0));
console.log('Presence reporting checks passed');
