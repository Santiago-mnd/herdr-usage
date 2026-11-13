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

2. **Cierre ordenado.** Al llegar una ventana al 90 %, pide a los panes de Claude que están trabajando, además del que acaba de terminar su turno, que hagan commit y push de su rama. También les pide una nota breve sobre dónde se quedaron y qué sigue. Envía esa petición una sola vez por ventana. Si falla la lista de agentes o el envío a un pane, vuelve a pedirle el cierre en el siguiente evento antes del reinicio; también alcanza a los panes que empiezan a trabajar después de la primera petición.

3. **Regreso.** Dos minutos después de la hora de reinicio, envía «retoma» a esos mismos panes, siempre que conserven la misma sesión de agente y estén en reposo (`idle` o `done`).

4. **Aviso para la persona.** Muestra un solo aviso de herdr por evento mediante `herdr notification show`, aunque haya varios panes involucrados.

Los mensajes dirigidos a los agentes están en español y comienzan con `[herdr-usage]`. El plugin no hace llamadas de red ni consulta servicios externos: obtiene el uso del JSON que Claude Code ya entrega a la statusline. Si varias sesiones escriben una lectura al mismo tiempo, conserva la ventana más reciente; dentro de una misma ventana, conserva el porcentaje más alto.

herdr ejecuta `herdr-usage hook` al arrancar y ante cada evento `pane.agent_status_changed`. El proceso lee el archivo de uso, toma una decisión y sale en unos 14 ms; no queda un daemon activo. Para un regreso pendiente crea una tarea de una sola vez con el programador del sistema: un timer de systemd de usuario en Linux, un LaunchAgent en macOS o una tarea del Programador de tareas en Windows. Todas se llaman `herdr-usage-resume-<sesión>-<hora>`; el timer de Linux se borra solo después de correr. En macOS, el LaunchAgent se guarda en `~/Library/LaunchAgents/dev.herdr-usage.herdr-usage-resume-<sesión>-<hora>.plist`: launchd lo ejecuta a la hora redondeada al minuto siguiente y la propia ejecución borra el archivo y lo descarga. En Windows, la tarea vive en `\herdr-usage\`: al correr intenta borrarse y, si permanece, vence una hora después de la hora programada. Si la tarea no corre, el siguiente evento de herdr después del reinicio hace el regreso; si falla el envío de «retoma» o herdr no responde, lo reintenta a los cinco minutos, hasta tres intentos en total. Cada sesión de herdr mantiene su propio estado.

## Requisitos

- Linux, macOS o Windows. El manifest declara las tres plataformas.
- herdr 0.9.5 o posterior.
- Rust con `cargo`. herdr compila el plugin en tu máquina durante la instalación con `cargo build --release --locked`; por eso `cargo` tiene que estar en el `PATH` de la terminal donde corres `herdr plugin install`. Una instalación normal con rustup agrega `~/.cargo/bin` al `PATH`. En Windows, la cadena predeterminada de Rust (MSVC) requiere Visual Studio Build Tools con C++; la cadena GNU falló al compilar por falta de `dlltool`.
- Claude Code.
- En Linux y macOS, `~/.local/bin` en el `PATH`. Si no está, usa la ruta completa del enlace en la configuración de la statusline. En Windows, la statusline debe apuntar a la ruta completa de `target\release\herdr-usage.exe` dentro de la carpeta de instalación del plugin.

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

Evalúa el hook e imprime los comandos de herdr y del programador del sistema sin correrlos:

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

Si quedó un regreso pendiente en Linux, consulta los timers y detén el correspondiente:

```sh
systemctl --user list-timers 'herdr-usage-*'
```

```sh
systemctl --user stop 'herdr-usage-resume-*.timer'
```

En macOS, consulta los LaunchAgents, descarga el correspondiente y borra su archivo `.plist`:

```sh
launchctl list | grep herdr-usage
```

```sh
launchctl bootout gui/$(id -u)/<etiqueta>
```

```sh
rm ~/Library/LaunchAgents/dev.herdr-usage.*.plist
```

En Windows, usa PowerShell para consultar las tareas pendientes y eliminarlas:

```powershell
Get-ScheduledTask -TaskPath '\herdr-usage\'
```

```powershell
Get-ScheduledTask -TaskPath '\herdr-usage\' | Unregister-ScheduledTask -Confirm:$false
```

Por último, elimina el enlace de la statusline en Linux o macOS:

```sh
rm ~/.local/bin/herdr-usage
```

## Límites conocidos

El plugin recibe una lectura nueva cuando una sesión interactiva de Claude Code obtiene una respuesta y redibuja su statusline; `claude -p` no ejecuta la statusline. Linux es el entorno de uso diario del autor, con herdr. En macOS 26 (Apple Silicon) y Windows 11 pasaron las pruebas unitarias; en cada sistema se probó una tarea de regreso real que corrió a su hora. En macOS se borró sola; en Windows, una tarea creada desde una sesión de administrador no pudo borrarse y la quitó el vencimiento de una hora. Todavía no se ha probado la instalación completa con herdr en esos dos sistemas. Por ahora admite Claude Code; Codex está en el plan.

En Windows, el plugin no crea el enlace estable `~/.local/bin/herdr-usage`. La statusline debe usar la ruta completa del binario instalado (`target\release\herdr-usage.exe` dentro de la carpeta del plugin); esa ruta cambia con cada `herdr plugin update`.

El plugin tampoco detecta si un pane se cortó por el límite de uso. Al reiniciarse la ventana, intenta retomar los panes a los que antes les pidió el cierre, sujeto a las condiciones de sesión y reposo descritas arriba.

Para el significado de los mensajes dirigidos a los agentes, consulta [docs/agents.md](docs/agents.md). Si vas a modificar el código del plugin, consulta [AGENTS.md](AGENTS.md).

## Créditos y licencia

La idea de leer los límites de uso de Claude Code y dibujarlos como barras viene de *sub-meter*, de Miguel Ángel Durán (midudev), incluido en [better-claude](https://github.com/midudev/better-claude) bajo licencia MIT. herdr-usage no usa su código. El plugin fue creado por Santiago García y se distribuye bajo licencia MIT; consulta [LICENSE](LICENSE).
