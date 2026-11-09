# AGENTS.md: herdr-usage

- Lenguaje: Rust (edition 2024). Es una excepción a la regla global de TypeScript, aprobada por el owner el 8-nov-2026. Nació como daemon; desde la v0.2.0 es un plugin de herdr que corre un instante por evento, y se quedó en Rust por el binario chico y el arranque rápido.
- Código en inglés; los textos que ven el owner y los agentes van en español.
- `herdr-plugin.toml`: manifest del plugin (startup y evento `pane.agent_status_changed`, los dos corren `herdr-usage hook`).
- `src/lib.rs`: lógica pura (lectura de la statusline y del archivo de uso, mezcla de lecturas, barra, decisión, a qué panes avisar, sesión, timer, textos) con pruebas. `src/main.rs`: entrada y salida (subcomandos hook, status y statusline; archivos, candado, herdr, systemd-run).
- Compilar con prioridad baja en el VPS: `nice -n 10 cargo build --release -j 2`.
- Pruebas: `cargo test`. Para probar en vivo, `hook --dry-run` o `statusline` con `--meter` apuntando a una copia y `HERDR_PLUGIN_STATE_DIR` a una carpeta temporal; nunca se edita a mano el archivo real de uso (`~/.local/state/herdr/plugins/herdr-usage/meter/claude.json`).
- sub-meter se desinstaló el 9-nov-2026; la statusline propia lo reemplaza.
- Nada se deja corriendo sin el visto bueno del owner (regla de procesos). El único proceso programado es el timer de regreso de una sola vez; el README dice cómo verlo y cancelarlo.
- Decisiones e historia: engram, `idea/herdr-usage-alerts` (proyecto knowledge-vault), y Linear SG-200.
