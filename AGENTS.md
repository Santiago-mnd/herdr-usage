# AGENTS.md: herdr-usage

- Lenguaje: Rust (edition 2024). Es una excepción a la regla global de TypeScript, aprobada por el owner el 8-nov-2026, porque es un daemon que vive todo el día (~3-5 MB contra ~40-60 MB de Node).
- Código en inglés; los textos que ven el owner y los agentes van en español.
- `src/lib.rs`: lógica pura (lectura de sub-meter, decisión, textos) con pruebas. `src/main.rs`: entrada y salida (archivos, herdr, inotify, timer).
- Compilar con prioridad baja en el VPS: `nice -n 10 cargo build --release -j 2`.
- Pruebas: `cargo test`. Para probar en vivo, `--dry-run` con `--meter` y `--state` apuntando a copias; nunca se edita el archivo real de sub-meter.
- Nada se deja corriendo sin el visto bueno del owner (regla de procesos). El servicio está en `contrib/` y el README dice cómo apagarlo.
- Decisiones e historia: engram, `idea/herdr-usage-alerts` (proyecto knowledge-vault).
