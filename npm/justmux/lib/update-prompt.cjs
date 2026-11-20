'use strict';

const readline = require('node:readline');

const CHOICES = Object.freeze([
  { label: 'Update', value: 'update' },
  { label: 'Skip', value: 'skip' },
  { label: 'Skip this version', value: 'skip-version' },
]);
const SIGNALS = Object.freeze({ SIGINT: 130, SIGTERM: 143, SIGHUP: 129, SIGQUIT: 131 });

/** An inline picker. All terminal changes end before npm or the Rust UI runs. */
function updatePrompt({ currentVersion, latestVersion }, runtime = process) {
  const input = runtime.stdin;
  const output = runtime.stdout;
  if (!input?.isTTY || !output?.isTTY || typeof input.setRawMode !== 'function') {
    return Promise.resolve('skip');
  }
  return new Promise((resolve) => {
    const wasRaw = input.isRaw === true;
    const wasFlowing = input.readableFlowing === true;
    let selected = 0;
    let height = 0;
    let changedRaw = false;
    let cursorHidden = false;
    let finished = false;
    const signalHandlers = new Map();
    const size = () => ({
      width: Math.max(0, (Number.isInteger(output.columns) && output.columns > 0 ? output.columns : 80) - 1),
      rows: Number.isInteger(output.rows) && output.rows > 0 ? output.rows : 24,
    });
    const moveToTop = (lines) => `\r${lines > 1 ? `\x1b[${lines - 1}A` : ''}`;
    const draw = () => {
      const { width, rows } = size();
      const options = CHOICES.map((choice, index) => ({
        text: `${index === selected ? '>' : ' '} ${index + 1}. ${choice.label}`,
        active: index === selected,
      }));
      const title = { text: '  JustMux update available' };
      const versions = { text: `  ${currentVersion} -> ${latestVersion}` };
      const footer = { text: '  Up/Down select | Enter confirm | Esc skip' };
      const blank = { text: '' };
      let lines;
      if (rows >= 8) lines = [title, versions, blank, ...options, blank, footer];
      else if (rows >= 6) lines = [title, versions, ...options, footer];
      else if (rows >= 5) lines = [versions, ...options, footer];
      else if (rows >= 4) lines = [versions, ...options];
      else {
        const start = Math.min(Math.max(0, selected - rows + 1), CHOICES.length - rows);
        lines = options.slice(start, start + rows);
      }
      // Never print into the final column: wrapping would invalidate the
      // relative cursor position, especially in a narrow tmux popup.
      const previousHeight = Math.min(height, rows);
      const nextHeight = Math.max(previousHeight, lines.length);
      const rendered = Array.from({ length: nextHeight }, (_, index) => {
        const line = lines[index] ?? blank;
        const text = line.text.slice(0, width);
        return `\x1b[2K${line.active ? `\x1b[7m${text}\x1b[0m` : text}`;
      });
      output.write(`${moveToTop(previousHeight)}${rendered.join('\r\n')}`);
      height = nextHeight;
    };
    const finish = (answer) => {
      if (finished) return;
      finished = true;
      input.removeListener('keypress', onKey);
      input.removeListener('end', onEnd);
      input.removeListener('close', onEnd);
      input.removeListener('error', onEnd);
      output.removeListener('resize', redraw);
      output.removeListener('error', onEnd);
      for (const [signal, handler] of signalHandlers) runtime.removeListener(signal, handler);
      try {
        const visibleHeight = Math.min(height, size().rows);
        if (visibleHeight) {
          output.write(`${moveToTop(visibleHeight)}${Array(visibleHeight).fill('\x1b[2K').join('\r\n')}${moveToTop(visibleHeight)}`);
        }
      } catch { /* Restoring input and the cursor must still run. */ }
      try { if (cursorHidden) output.write('\x1b[0m\x1b[?25h'); } catch { /* Output may have closed. */ }
      try { if (changedRaw) input.setRawMode(wasRaw); } catch { /* Input may have closed. */ }
      if (!wasFlowing) input.pause();
      resolve(answer);
    };
    const onEnd = () => finish('skip');
    const redraw = () => {
      if (finished) return;
      try { draw(); } catch { finish('skip'); }
    };
    const onKey = (_text, key = {}) => {
      if (finished) return;
      if (key.ctrl) {
        if (key.name === 'c') finish({ action: 'exit', code: 130 });
        else if (key.name === 'd') finish('skip');
        return;
      }
      // Node marks a standalone Esc as meta:true as well as Alt shortcuts.
      if (key.meta && key.name !== 'escape') return;
      switch (key.name) {
        case 'up': case 'k':
          selected = (selected + CHOICES.length - 1) % CHOICES.length;
          redraw();
          break;
        case 'down': case 'j': case 'tab':
          selected = (selected + 1) % CHOICES.length;
          redraw();
          break;
        case 'return': case 'enter':
          finish(CHOICES[selected].value);
          break;
        case 'escape':
          finish('skip');
          break;
        default: {
          const choice = CHOICES[Number(key.name) - 1];
          if (choice) finish(choice.value);
        }
      }
    };
    try {
      for (const [signal, code] of Object.entries(SIGNALS)) {
        const handler = () => finish({ action: 'exit', code });
        signalHandlers.set(signal, handler);
        runtime.on(signal, handler);
      }
      input.on('keypress', onKey);
      input.on('end', onEnd);
      input.on('close', onEnd);
      input.on('error', onEnd);
      output.on('resize', redraw);
      output.on('error', onEnd);
      readline.emitKeypressEvents(input);
      if (!wasRaw) {
        changedRaw = true;
        input.setRawMode(true);
      }
      cursorHidden = true;
      output.write('\x1b[?25l');
      draw();
      input.resume();
    } catch { finish('skip'); }
  });
}

module.exports = { updatePrompt };
