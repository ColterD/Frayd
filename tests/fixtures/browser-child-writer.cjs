#!/usr/bin/env node
'use strict';
// Controllable lifecycle fixture: a launcher with a child that ignores TERM and
// keeps writing its owned disposable profile after the launcher has exited.
const { spawn } = require('node:child_process');
if (process.argv.includes('--version')) {
  console.log('Frayd controlled child-writer fixture');
} else {
  const profile = process.argv.find(arg => arg.startsWith('--user-data-dir=')).slice('--user-data-dir='.length);
  const child = spawn(process.execPath, ['-e', `
    const fs = require('node:fs');
    const path = require('node:path');
    process.on('SIGTERM', () => {});
    function write() {
      if (process.argv[2] && fs.existsSync(path.join(process.argv[2], 'writer.pause'))) {
        fs.writeFileSync(path.join(process.argv[2], 'writer.paused'), 'paused');
        return;
      }
      fs.mkdirSync(path.join(process.argv[1], 'Default'), { recursive: true });
      const writer = path.join(process.argv[1], 'Default', 'writer');
      fs.writeFileSync(writer, String(Date.now()));
    }
    write();
    setInterval(write, 5);
    process.send('ready');
  `, profile, process.env.FRAYD_CHILD_FIXTURE_CONTROL_DIR || ''],
  { stdio: ['ignore', 'ignore', 'inherit', 'ipc'] });
  child.once('message', async () => {
    if (process.env.FRAYD_CHILD_FIXTURE_EARLY_EXIT === '1') {
      console.error('original browser exit sentinel');
      process.exit(7);
    }
    const url = process.argv[process.argv.length - 1];
    await fetch(new URL('/__fixture/result', url), { method: 'POST',
      body: JSON.stringify({ cases: [{ name: 'controlled original failure', status: 'FAIL', error: 'original failure sentinel' }],
        errors: ['original failure sentinel'], passed: 0, failed: 1, writerPid: child.pid, profile }) });
  });
  process.on('SIGTERM', () => process.exit(0));
}
