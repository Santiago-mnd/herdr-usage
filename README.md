# herdr-usage

[Leer en español](README.es.md)

herdr-usage is an early-stage, Rust-based plugin for [herdr](https://herdr.dev), a terminal multiplexer for coding agents. Santiago García uses version 0.2.0 daily on a Linux VPS with several Claude Code panes running at once.

When a Claude plan’s 5-hour or weekly usage window runs out, agents can stop midway through a task. herdr-usage asks them to leave a handoff before the limit, then prompts those same panes to resume after the window resets.

## How it works

The plugin has four parts:

1. **Usage bar.** `herdr-usage statusline` is a Claude Code statusline. It reads the `rate_limits` JSON Claude Code sends through stdin, saves the reading to a file, and draws a bar for both the 5-hour and weekly windows. Each bar shows its percentage and time until reset. The color is Claude orange through 69%, amber from 70%, and red from 90%.

   Without colors, a reading looks like this:

   ```text
   5 h ▰▱▱▱▱▱▱▱ 11 % (reinicia en 1 h 1 min)  ·  sem ▱▱▱▱▱▱▱▱ 5 % (reinicia en 6 d 7 h)
   ```

   The bars stretch to fit the terminal width that Claude Code passes in `COLUMNS`, from 6 to 30 cells each, leaving 10 columns free for Claude Code's own notices. On a narrow terminal the text shortens to `(1 h 1 min)`. The bar resizes on the next redraw, not when you resize the window.

   The bar labels and agent messages are in Spanish; `sem` means *weekly*.

2. **Handoff.** At 90%, the plugin asks working Claude panes, plus the pane that just finished its turn, to commit and push their branches. It also asks each agent to leave a short note about where it stopped and what comes next. It sends this request once per usage window.

3. **Resume.** Two minutes after the reset, the plugin sends `retoma` (“resume”) to those same panes. It does so only when a pane still has the same agent session and its status is `idle` or `done`.

4. **Human notification.** Each event produces one `herdr notification show` notice for you, regardless of how many panes are involved.

Agent messages begin with `[herdr-usage]`. See [docs/agents.md](docs/agents.md) for what those messages mean and what an agent is expected to do.

There is no daemon. herdr runs `herdr-usage hook` when it starts and on each `pane.agent_status_changed` event; the process reads the saved usage, decides what to do, and exits in about 14 ms. For a pending resume, it creates one single-use systemd user timer named `herdr-usage-resume-<session>-<time>`, which removes itself afterward. Each herdr session keeps its own state.

The plugin makes no network calls or requests to external services. Its usage data comes from the JSON Claude Code already passes to the statusline. If multiple Claude sessions write readings at once, the plugin keeps the most recent window and, within that window, the highest percentage.

## Requirements

- Linux with systemd user services (`systemctl --user`). macOS is not supported yet.
- herdr 0.9.5 or newer.
- Rust and `cargo`. herdr builds the plugin locally with `cargo build --release --locked` during installation, so `cargo` must be on the `PATH` of the shell where you run `herdr plugin install`. A standard rustup install adds `~/.cargo/bin` to it.
- Claude Code.
- `~/.local/bin` on your `PATH`, or the full path to the link in your statusline command.

## Install

1. Install the plugin:

   ```sh
   herdr plugin install Santiago-mnd/herdr-usage
   ```

2. Let herdr run it once, either when the herdr server starts or when an agent’s status first changes. The plugin creates `~/.local/bin/herdr-usage`; check the link with:

   ```sh
   ls -l ~/.local/bin/herdr-usage
   ```

3. Add this `statusLine` entry to `~/.claude/settings.json`:

   ```json
   "statusLine": {
     "type": "command",
     "command": "herdr-usage statusline",
     "padding": 0
   }
   ```

   If `~/.local/bin` is absent from your `PATH`, use the link’s full path as the `command` value. Claude Code picks up the settings change without a restart. The bar appears after the next response; before a session’s first response, it shows the last saved reading or `uso del plan: sin datos todavía`.

Each time herdr runs the plugin, it creates or updates the link to the currently installed binary. This keeps the statusline working after `herdr plugin update`. If `~/.local/bin/herdr-usage` already exists as a regular file, the plugin leaves it alone.

## Commands

Inspect saved readings, state, and the action the plugin would take:

```sh
herdr-usage status
```

Evaluate the hook and print its herdr and systemd commands without running them:

```sh
herdr-usage hook --dry-run
```

Show command help:

```sh
herdr-usage --help
```

View the log for each plugin execution:

```sh
herdr plugin log list --plugin herdr-usage
```

The hook defaults to a 90% threshold and a two-minute reset margin. Its `--threshold` and `--margin` options can change those values, though there is not yet a convenient way to configure them for the installed plugin.

## Uninstall

Remove the plugin:

```sh
herdr plugin uninstall herdr-usage
```

Remove the `statusLine` block from `~/.claude/settings.json`. If a resume is pending, list its timers, then stop the pending timer:

```sh
systemctl --user list-timers 'herdr-usage-*'
```

```sh
systemctl --user stop 'herdr-usage-resume-*.timer'
```

Finally, remove the statusline link:

```sh
rm ~/.local/bin/herdr-usage
```

## Known limits

The plugin learns about usage only when an interactive Claude Code session receives a response and redraws its statusline; `claude -p` does not run it. Resume currently requires Linux and systemd, and Claude is the only supported agent for now. Codex support is planned.

The plugin does not detect whether a pane actually stopped because of a usage limit. It resumes the panes it previously asked to hand off, subject to the session and status checks above.

## Credits and license

The idea of reading Claude Code rate limits and drawing them as a bar comes from Miguel Ángel Durán’s *sub-meter*, part of [better-claude](https://github.com/midudev/better-claude) (MIT). herdr-usage does not use its code. The plugin is built for [herdr](https://herdr.dev).

herdr-usage is by Santiago García and is licensed under MIT; see [LICENSE](LICENSE). For code changes, read [AGENTS.md](AGENTS.md). The repository is [Santiago-mnd/herdr-usage](https://github.com/Santiago-mnd/herdr-usage).
