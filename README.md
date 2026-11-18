# herdr-usage

[Leer en español](README.es.md)

herdr-usage is an early-stage, Rust-based plugin for [herdr](https://herdr.dev), a terminal multiplexer for coding agents. Santiago García uses version 0.2.0 daily on a Linux VPS with several Claude Code panes running at once.

When a Claude Code or Codex CLI plan’s usage window runs out, agents can stop midway through a task. herdr-usage asks them to leave a handoff before the limit, then prompts those same panes to resume after the window resets.

## How it works

The plugin has four parts:

1. **Usage bar.** `herdr-usage statusline` is a Claude Code statusline. It reads the `rate_limits` JSON Claude Code sends through stdin, saves the reading to `meter/claude.json` in the plugin’s state directory, and draws a bar for both the 5-hour and weekly windows. Each bar shows its percentage and time until reset. The color is Claude orange through 69%, amber from 70%, and red from 90%.

   Without colors, a reading looks like this:

   ```text
   5 h ▰▱▱▱▱▱▱▱ 11 % (reinicia en 1 h 1 min)  ·  sem ▱▱▱▱▱▱▱▱ 5 % (reinicia en 6 d 7 h)
   ```

   The bars stretch to fit the terminal width that Claude Code passes in `COLUMNS`, from 6 to 30 cells each, leaving 10 columns free for Claude Code's own notices. On a narrow terminal the text shortens to `(1 h 1 min)`. The bar resizes on the next redraw, not when you resize the window.

   The bar labels and agent messages are in Spanish; `sem` means *weekly*. Codex CLI’s statusline accepts only built-in elements, so there is no Codex usage bar; `herdr-usage statusline` works only with Claude Code.

2. **Handoff.** At 90%, the plugin asks working panes of the affected agent, plus the pane that just finished its turn, to commit and push their branches. It also asks each agent to leave a short note about where it stopped and what comes next. It sends this request once per usage window. If it could not list agents or send the request to a pane, it asks again on the next event before the reset; this also covers panes that start working after the first request. A Claude limit affects only Claude panes, and a Codex limit affects only Codex panes.

3. **Resume.** Two minutes after the reset, the plugin sends `retoma` (“resume”) to those same panes. It does so only when a pane still has the same agent session and its status is `idle` or `done`. If sending the message fails or herdr does not respond, it retries after five minutes, up to three attempts in total. Claude and Codex have separate resume tasks.

4. **Human notification.** Each event produces one `herdr notification show` notice for you, regardless of how many panes are involved. The notice names the agent, such as “Uso semanal de Codex cerca del límite” or “Se reinició el uso de 5 h de Claude.”

Agent messages begin with `[herdr-usage]`. See [docs/agents.md](docs/agents.md) for what those messages mean and what an agent is expected to do.

There is no daemon. herdr runs `herdr-usage hook` when it starts and on each `pane.agent_status_changed` event; the process reads usage, decides what to do, and exits. For a pending resume, the plugin creates a one-time task using a systemd user timer on Linux, a LaunchAgent on macOS, or Task Scheduler on Windows. Claude tasks are named `herdr-usage-resume-<session>-<time>`; Codex tasks are named `herdr-usage-codex-resume-<session>-<time>`. Each herdr session keeps separate state for each agent: `<plugin state>/<session>.json` for Claude and `<plugin state>/codex/<session>.json` for Codex.

On Linux, the timer removes itself after running. On macOS, the Claude LaunchAgent lives at `~/Library/LaunchAgents/dev.herdr-usage.herdr-usage-resume-<session>-<time>.plist`; the Codex LaunchAgent lives at `~/Library/LaunchAgents/dev.herdr-usage.herdr-usage-codex-resume-<session>-<time>.plist`. launchd runs each one at the scheduled time, rounded up to the next minute, and the run unloads it and removes the file. On Windows, both agents’ tasks live in Task Scheduler’s `\herdr-usage\` folder; each task attempts to remove itself after running and expires one hour after its scheduled time. If a task does not run because the machine was off or the task was removed, the next herdr event after the reset triggers the resume.

The plugin makes no network calls or requests to external services. Claude usage comes from the JSON Claude Code already passes to the statusline. If multiple Claude sessions write readings at once, the plugin keeps the most recent window and, within that window, the highest percentage.

Codex writes a `token_count` event containing `rate_limits` after each response to a session file under `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, or `$CODEX_HOME/sessions` when that variable is set. `codex exec` writes these files too, except when run with `--ephemeral`. On each plugin run, herdr-usage checks folders for the seven most recent days, selects the three most recently modified session files, and reads the last MiB of each. It saves the latest reading to `meter/codex.json` in the plugin’s state directory. If the Codex sessions directory does not exist, the Codex part does nothing; no configuration is needed to enable it.

Codex identifies usage windows by duration in minutes: `300` is the 5-hour window, `10080` is weekly, and `43200` is 30 days. Other durations are labeled in days or minutes. The plugin uses whichever windows Codex reports for the plan and treats two readings as the same window when their reset times differ by less than five minutes.

## Requirements

- Linux, macOS, or Windows. The plugin manifest declares all three platforms.
- herdr 0.9.5 or newer.
- Rust and `cargo`. herdr builds the plugin locally with `cargo build --release --locked` during installation, so `cargo` must be on the `PATH` of the shell where you run `herdr plugin install`. A standard rustup install adds `~/.cargo/bin` to it. On Windows, the default Rust MSVC toolchain also needs Visual Studio Build Tools with C++.
- Claude Code, Codex CLI, or both. Codex support was tested with Codex CLI 0.170.0.
- For the Claude Code usage bar on Linux and macOS, `~/.local/bin` on your `PATH`, or the full path to the link in your statusline command. On Windows, point the statusline to the installed binary’s full path, `target\release\herdr-usage.exe` inside the plugin installation directory. That path changes with each `herdr plugin update`.

## Install

1. Install the plugin:

   ```sh
   herdr plugin install Santiago-mnd/herdr-usage
   ```

2. Let herdr run it once, either when the herdr server starts or when an agent’s status first changes. The plugin creates `~/.local/bin/herdr-usage`; check the link with:

   ```sh
   ls -l ~/.local/bin/herdr-usage
   ```

3. To show the Claude Code usage bar, add this `statusLine` entry to `~/.claude/settings.json`:

   ```json
   "statusLine": {
     "type": "command",
     "command": "herdr-usage statusline",
     "padding": 0
   }
   ```

   If `~/.local/bin` is absent from your `PATH`, use the link’s full path as the `command` value. Claude Code picks up the settings change without a restart. The bar appears after the next response; before a session’s first response, it shows the last saved reading or `uso del plan: sin datos todavía`.

Each time herdr runs the plugin, it creates or updates the link to the currently installed binary. This keeps the Claude Code statusline working after `herdr plugin update`. If `~/.local/bin/herdr-usage` already exists as a regular file, the plugin leaves it alone.

## Commands

Inspect saved readings, state, and the action the plugin would take:

```sh
herdr-usage status
```

`status` shows a section for each agent, `[Claude]` and `[Codex]`. Use `--provider <claude|codex>` with `status` or `hook` to evaluate one agent; without it, both are evaluated. For compatibility, `--meter` and `--state` refer to Claude when `--provider` is omitted.

Evaluate the hook and print its herdr and system scheduler commands without running them:

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

The hook defaults to a 90% threshold and a two-minute reset margin for both agents. Its `--threshold` and `--margin` options can change those values, though there is not yet a convenient way to configure them for the installed plugin.

## Uninstall

Remove the plugin:

```sh
herdr plugin uninstall herdr-usage
```

Remove the `statusLine` block from `~/.claude/settings.json` if you added it. If a resume is pending, use the commands for your system to find and remove its scheduled task.

On Linux, list the timers:

```sh
systemctl --user list-timers 'herdr-usage-*'
```

Then stop any pending resume timer for either agent:

```sh
systemctl --user stop 'herdr-usage-*resume-*.timer'
```

On macOS, list the LaunchAgents:

```sh
launchctl list | grep herdr-usage
```

Unload each pending LaunchAgent, replacing `<label>` with its label:

```sh
launchctl bootout gui/$(id -u)/<label>
```

Then remove its `.plist` file:

```sh
rm ~/Library/LaunchAgents/dev.herdr-usage.*.plist
```

On Windows, run these commands in PowerShell to list the pending tasks:

```powershell
Get-ScheduledTask -TaskPath '\herdr-usage\'
```

Then unregister them:

```powershell
Get-ScheduledTask -TaskPath '\herdr-usage\' | Unregister-ScheduledTask -Confirm:$false
```

Finally, remove the statusline link on Linux or macOS:

```sh
rm ~/.local/bin/herdr-usage
```

## Known limits

The plugin learns about Claude usage only when an interactive Claude Code session receives a response and redraws its statusline; `claude -p` does not run it. It learns about Codex usage only after Codex writes a reading to a local session file following a response.

Linux on the author’s VPS is the daily-use setup with Claude Code and herdr. On macOS 26 (Apple Silicon) and Windows 11, the unit tests passed, and a real scheduled Claude resume task ran on time. On macOS it removed itself; on Windows, a task created from an administrator session could not remove itself and was removed by the one-hour expiry instead. Full installation with herdr has not yet been tested on either system.

Codex support was tested on the author’s Linux VPS with unit tests, plus `hook --dry-run` against copies of Codex session files for handoff and resume, and `status` against real session files for usage readings. A real Codex handoff with herdr has not yet been tested, nor has Codex support on macOS or Windows. On Windows, the stable `~/.local/bin/herdr-usage` link is not created, so the Claude Code statusline command must use the installed binary’s full path and be updated after each `herdr plugin update`.

The plugin does not detect whether a pane actually stopped because of a usage limit. It resumes the panes it previously asked to hand off, subject to the session and status checks above.

## Credits and license

The idea of reading Claude Code rate limits and drawing them as a bar comes from Miguel Ángel Durán’s *sub-meter*, part of [better-claude](https://github.com/midudev/better-claude) (MIT). herdr-usage does not use its code. The plugin is built for [herdr](https://herdr.dev).

herdr-usage is by Santiago García and is licensed under MIT; see [LICENSE](LICENSE). For code changes, read [AGENTS.md](AGENTS.md). The repository is [Santiago-mnd/herdr-usage](https://github.com/Santiago-mnd/herdr-usage).
