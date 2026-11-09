# herdr-usage

Plugin de herdr en Rust que mide el uso del plan de Claude y actúa sobre los panes. El uso sale de la statusline oficial de Claude Code (`rate_limits` del JSON que recibe por stdin); no llama a ningún servicio externo.

## Qué hace

0. **Barra de uso.** `herdr-usage statusline` es la statusline de Claude Code: dibuja la ventana de 5 h y la semanal, con lo que falta para el reinicio, y guarda la lectura en `<estado del plugin>/meter/claude.json`. Si varias sesiones de Claude escriben a la vez, se queda con la ventana más reciente y, dentro de la misma ventana, con el porcentaje más alto.
1. **Cierre ordenado.** Cuando la ventana de 5 h o la semanal llega al umbral (90 % por defecto), les pide a los panes de Claude que están trabajando, y al que acaba de terminar su turno, que hagan commit y push y que dejen una nota de dónde se quedaron. Lo pide una sola vez por ventana, sin importar cuántos panes haya.
2. **Regreso.** A la hora del reinicio (más 2 min de margen) les dice «retoma» a esos mismos panes, siempre que sigan con la misma sesión de agente y estén en `idle` o `done`.
3. **Un solo aviso para ti** en cada evento, con `herdr notification show`; nunca uno por pane.

## Cómo corre

No hay daemon. herdr lanza `herdr-usage hook` en cada `pane.agent_status_changed` y al arrancar el servidor. El proceso lee el archivo de uso, decide y sale. La statusline actualiza ese archivo con cada respuesta de Claude, así que al terminar un turno la lectura está fresca.

Para el regreso, cuando pide un cierre arma **un solo timer de systemd de una sola vez** (`herdr-usage-resume-<sesión>-<hora>`), que corre `herdr-usage hook` a la hora del reinicio y se borra solo. No llama a ningún servicio externo.

Cada sesión de herdr lleva su propio estado en `$HERDR_PLUGIN_STATE_DIR/<sesión>.json` (en Linux, `~/.local/state/herdr/plugins/herdr-usage/`). Un candado de archivo evita que dos eventos simultáneos dupliquen el aviso.

## Instalar en local

Requiere Linux con systemd de usuario y `cargo`.

```bash
cargo build --release
herdr plugin link .
```

En `~/.claude/settings.json`:

```json
"statusLine": {
  "type": "command",
  "command": "/ruta/a/herdr-usage/target/release/herdr-usage statusline",
  "padding": 0
}
```

```bash
herdr plugin log list --plugin herdr-usage     # bitácora de cada ejecución
```

Para apagarlo:

```bash
herdr plugin unlink herdr-usage
systemctl --user list-timers 'herdr-usage-*'   # si quedó un regreso pendiente
systemctl --user stop 'herdr-usage-resume-*.timer'
```

## Comandos

```bash
./target/release/herdr-usage status            # lecturas, estado y lo que haría
./target/release/herdr-usage hook --dry-run    # evalúa e imprime los comandos sin correrlos
./target/release/herdr-usage --help
```

## Límites conocidos

- Solo sabe del uso cuando alguna sesión interactiva de Claude recibe una respuesta y redibuja su statusline. `claude -p` no la corre.
- La statusline y el evento de fin de turno llegan casi juntos; si el hook lee antes, la lectura nueva se toma en el siguiente evento.
- Solo Linux por ahora: el timer de regreso usa systemd.
- No detecta si un pane se cortó por límite. Retoma los panes a los que les pidió el cierre.
