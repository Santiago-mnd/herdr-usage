# AGENTS.md: herdr-usage

- Lenguaje: Rust (edition 2024). Es una excepción a la regla global de TypeScript, aprobada por el owner el 8-nov-2026. Nació como daemon; desde la v0.2.0 es un plugin de herdr que corre un instante por evento, y se quedó en Rust por el binario chico y el arranque rápido.
- Código en inglés; los textos que ven el owner y los agentes van en español.
- `herdr-plugin.toml`: manifest del plugin (startup y evento `pane.agent_status_changed`, los dos corren `herdr-usage hook`).
- `src/lib.rs`: lógica pura (lectura de la statusline y del archivo de uso, mezcla de lecturas, barra, decisión, a qué panes avisar, sesión, timer, textos) con pruebas. `src/main.rs`: entrada y salida (subcomandos hook, status y statusline; archivos, candado, herdr y el programador de cada sistema). `src/schedule.rs`: textos puros de las tareas de regreso (launchd y Programador de tareas) con pruebas.
- Compilar con prioridad baja en el VPS: `nice -n 10 cargo build --release -j 2`.
- Pruebas: `cargo test`. Para probar en vivo, `hook --dry-run` o `statusline` con `--meter` apuntando a una copia y `HERDR_PLUGIN_STATE_DIR` a una carpeta temporal; nunca se edita a mano el archivo real de uso (`~/.local/state/herdr/plugins/herdr-usage/meter/claude.json`).
- sub-meter se desinstaló el 9-nov-2026; la statusline propia lo reemplaza.
- Cada ejecución lanzada por herdr (con `HERDR_PLUGIN_ID`) mantiene el enlace `~/.local/bin/herdr-usage` hacia el binario de la instalación actual, para que la `statusLine` de Claude Code no dependa de la carpeta que herdr cambia en cada `update`.
- Documentación pública: `README.md` (inglés) y `README.es.md` (español), con los mismos hechos; `docs/agents.md` es para los agentes que reciben los mensajes `[herdr-usage]`. Se redactan con la skill `writing` y se actualizan juntos.
- Nada se deja corriendo sin el visto bueno del owner (regla de procesos). El único proceso programado es la tarea de regreso de una sola vez (systemd en Linux, launchd en macOS, Programador de tareas en Windows); el README dice cómo verla y cancelarla.
- Decisiones e historia: engram, `idea/herdr-usage-alerts` (proyecto knowledge-vault), y Linear SG-200.
- Probar en Windows (`win`) desde el VPS: rustup GNU nativo falla con «error calling dlltool», así que se compila aquí con `gcc-mingw-w64-x86-64` (`cargo build --target x86_64-pc-windows-gnu` con `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc`) y se copian el `.exe` y el binario de pruebas a `%USERPROFILE%\herdr-usage-test\bin`. Una tarea creada desde SSH (sesión elevada) no puede borrarse sola; la quita su vencimiento a la hora. En la Mac (`mac`), rustup viene de Homebrew en `/opt/homebrew/opt/rustup/bin` y la copia de prueba vive en `~/herdr-usage-test` (13-nov-2026).
