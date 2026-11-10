# Messages for agents in herdr panes

If you receive a message beginning with `[herdr-usage]`, follow the steps below. These messages come from herdr-usage, a herdr plugin that coordinates work around Claude plan usage resets. You can link this file from your `AGENTS.md` or `CLAUDE.md`.

## When the usage window is nearly over

You may receive this message:

> [herdr-usage] El uso del plan va {detalle}. Antes del corte, cierra lo que llevas: haz commit y push de tu rama y deja una nota breve de dónde te quedaste y qué sigue. No empieces nada nuevo; cuando se reinicie el uso te aviso para que retomes.

`{detalle}` describes the usage level, such as `de 5 h al 91 %` or `de 5 h al 92 % y semanal al 90 %`.

1. Finish the current step, or pause it at a stable point.
2. Commit the work on your current branch, then push it. If the push fails, keep the local commit and report the failure in your response.
3. In that same response, leave a short note stating where you stopped, what to do next, and which files or commands matter.
4. Start no new work. Remain idle until you receive a resume message.

If you have no work in progress, say so and remain idle. The plugin does not read your note; it is there for you or a person to use when work resumes.

## When usage resets

You may receive this message:

> [herdr-usage] Ya se reinició el uso del plan. Retoma donde te quedaste, según la nota que dejaste antes del corte.

1. Reread the note you left before the cutoff.
2. Check the repository’s current branch and uncommitted changes.
3. Continue with the next step recorded in your note.

The resume message arrives only if your pane still has the same agent session and is idle or done. In the author’s demo, a Claude Haiku agent committed its work, reported that its push failed because the repository had no remote, and continued from its note after receiving the resume message.
