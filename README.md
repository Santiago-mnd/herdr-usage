# herdr-usage

Daemon en Rust que vigila el uso del plan de Claude y actúa sobre los panes de herdr. Lee el archivo que deja el plugin `sub-meter@better-claude` en `~/.claude/plugins/store/`.

## Qué hace (v1)

1. **Cierre ordenado.** Cuando la ventana de 5 h o la semanal llega al umbral (90 % por defecto), les pide a los panes de Claude que están trabajando que hagan commit y push y que dejen una nota de dónde se quedaron. Lo pide una sola vez por ventana, sin importar cuántos panes haya ni cuántas veces cambie el archivo.
2. **Regreso.** A la hora del reinicio (más 2 min de margen) les dice «retoma» a esos mismos panes, siempre que sigan con la misma sesión de agente y estén en `idle` o `done`.
3. **Un solo aviso para ti** en cada evento, con `herdr notification show`; nunca uno por pane.

No hace polling: vigila el archivo con inotify y duerme con un timer hasta el reinicio. No llama a ningún servicio externo.

## Comandos

```bash
cargo build --release
./target/release/herdr-usage --status            # lecturas, estado y lo que haría
./target/release/herdr-usage --once --dry-run    # evalúa una vez e imprime los comandos sin correrlos
./target/release/herdr-usage --help
```

Estado: `~/.local/state/herdr-usage/state.json`.

## Dejarlo corriendo (pendiente de aprobación del owner)

No está instalado ni corriendo. Para activarlo:

```bash
install -m 755 target/release/herdr-usage ~/.local/bin/
mkdir -p ~/.config/systemd/user
cp contrib/herdr-usage.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now herdr-usage
journalctl --user -u herdr-usage -f              # bitácora
```

Para apagarlo:

```bash
systemctl --user disable --now herdr-usage
```

## Límites conocidos

- Solo sabe del uso cuando sub-meter actualiza su archivo, y eso pasa cuando alguna sesión de Claude lee el plan.
- No detecta si un pane se cortó por límite. Retoma los panes a los que les pidió el cierre.
- Pendiente para la v2: el pronóstico del ritmo de consumo.
