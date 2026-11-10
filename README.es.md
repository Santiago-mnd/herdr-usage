# herdr-usage

[Read in English](README.md)

**herdr-usage** es un plugin de [herdr](https://herdr.dev), el multiplexor de terminal para agentes de código. Está escrito en Rust, va en la versión 0.2.0 y se encuentra en una etapa temprana; su autor lo usa a diario en un VPS Linux con varios panes de Claude Code abiertos a la vez.

Cuando se agota la ventana de uso de cinco horas o la semanal del plan de Claude, un agente puede quedarse a media tarea. El plugin muestra cuánto uso queda, pide a los agentes que preparen un cierre al llegar al umbral y programa su regreso después del reinicio.

## Cómo funciona

El plugin reúne cuatro funciones:

1. **Barra de uso.** `herdr-usage statusline` recibe por stdin los `rate_limits` que Claude Code entrega a su statusline. Guarda la lectura en un archivo y muestra dos barras, una para la ventana de cinco horas y otra para la semanal, con el porcentaje usado y el tiempo restante hasta el reinicio. Usa el naranja de Claude hasta el 69 %, ámbar desde el 70 % y rojo desde el 90 %. Sin colores, se ve así:

   ```text
   5 h ▰▱▱▱▱▱▱▱ 11 % (reinicia en 1 h 1 min)  ·  sem ▱▱▱▱▱▱▱▱ 5 % (reinicia en 6 d 7 h)
   ```

   Las barras se ajustan al ancho de la terminal que Claude Code entrega en `COLUMNS`: miden de 6 a 30 celdas cada una y dejan 10 columnas libres para los avisos de Claude Code. En una terminal angosta, el texto se acorta a `(1 h 1 min)`; además, la barra cambia de tamaño en el siguiente redibujado, no al momento de redimensionar la ventana.

2. **Cierre ordenado.** Al llegar una ventana al 90 %, pide a los panes de Claude que están trabajando, además del que acaba de terminar su turno, que hagan commit y push de su rama. También les pide una nota breve sobre dónde se quedaron y qué sigue. Envía esa petición una sola vez por ventana.

3. **Regreso.** Dos minutos después de la hora de reinicio, envía «retoma» a esos mismos panes, siempre que conserven la misma sesión de agente y estén en reposo (`idle` o `done`).

4. **Aviso para la persona.** Muestra un solo aviso de herdr por evento mediante `herdr notification show`, aunque haya varios panes involucrados.

Los mensajes dirigidos a los agentes están en español y comienzan con `[herdr-usage]`. El plugin no hace llamadas de red ni consulta servicios externos: obtiene el uso del JSON que Claude Code ya entrega a la statusline. Si varias sesiones escriben una lectura al mismo tiempo, conserva la ventana más reciente; dentro de una misma ventana, conserva el porcentaje más alto.

herdr ejecuta `herdr-usage hook` al arrancar y ante cada evento `pane.agent_status_changed`. El proceso lee el archivo de uso, toma una decisión y sale en unos 14 ms; no queda un daemon activo. Para el regreso crea un único timer de systemd de usuario, de una sola vez, llamado `herdr-usage-resume-<sesión>-<hora>`; el timer se borra solo. Cada sesión de herdr mantiene su propio estado.

## Requisitos

- Linux con systemd de usuario (`systemctl --user`); macOS todavía no está disponible.
- herdr 0.9.5 o posterior.
- Rust con `cargo`. herdr compila el plugin en tu máquina durante la instalación con `cargo build --release --locked`.
- Claude Code.
- `~/.local/bin` en el `PATH`. Si no está, usa la ruta completa del enlace en la configuración de la statusline.

## Instalación

1. Instala el plugin:

   ```sh
   herdr plugin install Santiago-mnd/herdr-usage
   ```

2. Espera a que herdr lo ejecute al arrancar su servidor o cuando cambie por primera vez el estado de un agente. Esa ejecución crea `~/.local/bin/herdr-usage`; comprueba el enlace con:

   ```sh
   ls -l ~/.local/bin/herdr-usage
   ```

3. Agrega este bloque a `~/.claude/settings.json`:

   ```json
   "statusLine": {
     "type": "command",
     "command": "herdr-usage statusline",
     "padding": 0
   }
   ```

Claude Code toma el cambio sin reiniciar y muestra la barra tras la siguiente respuesta. Antes de la primera respuesta de una sesión, muestra la última lectura guardada o «uso del plan: sin datos todavía».

Cada vez que herdr ejecuta el plugin, este crea o actualiza `~/.local/bin/herdr-usage` para apuntar al binario instalado. Así, la statusline sigue funcionando después de `herdr plugin update`. Si en esa ruta existe un archivo que no es un enlace, el plugin lo deja intacto.

## Comandos útiles

Consulta las lecturas, el estado y lo que haría el plugin, sin ejecutar acciones:

```sh
herdr-usage status
```

Evalúa el hook e imprime los comandos de herdr y systemd sin correrlos:

```sh
herdr-usage hook --dry-run
```

Consulta la ayuda:

```sh
herdr-usage --help
```

Revisa la bitácora de cada ejecución:

```sh
herdr plugin log list --plugin herdr-usage
```

El umbral predeterminado es 90 % y el margen tras el reinicio es de dos minutos. El subcomando `hook` acepta `--threshold` y `--margin`; todavía no hay una forma cómoda de configurarlos para el plugin instalado.

## Desinstalación

Desinstala el plugin y quita el bloque `statusLine` de `~/.claude/settings.json`:

```sh
herdr plugin uninstall herdr-usage
```

Si quedó un regreso pendiente, consulta los timers y detén el correspondiente:

```sh
systemctl --user list-timers 'herdr-usage-*'
```

```sh
systemctl --user stop 'herdr-usage-resume-*.timer'
```

Por último, elimina el enlace de la statusline:

```sh
rm ~/.local/bin/herdr-usage
```

## Límites conocidos

El plugin recibe una lectura nueva cuando una sesión interactiva de Claude Code obtiene una respuesta y redibuja su statusline; `claude -p` no ejecuta la statusline. El regreso depende de systemd, por lo que funciona solo en Linux. Por ahora admite Claude Code; Codex está en el plan.

El plugin tampoco detecta si un pane se cortó por el límite de uso. Al reiniciarse la ventana, intenta retomar los panes a los que antes les pidió el cierre, sujeto a las condiciones de sesión y reposo descritas arriba.

Para el significado de los mensajes dirigidos a los agentes, consulta [docs/agents.md](docs/agents.md). Si vas a modificar el código del plugin, consulta [AGENTS.md](AGENTS.md).

## Créditos y licencia

La idea de leer los límites de uso de Claude Code y dibujarlos como barras viene de *sub-meter*, de Miguel Ángel Durán (midudev), incluido en [better-claude](https://github.com/midudev/better-claude) bajo licencia MIT. herdr-usage no usa su código. El plugin fue creado por Santiago García y se distribuye bajo licencia MIT; consulta [LICENSE](LICENSE).
