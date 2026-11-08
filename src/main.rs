use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::mpsc;
use std::time::Duration as StdDuration;

use chrono::{Duration, Local, Utc};
use herdr_usage::{
    Action, Limit, PaneRef, RESUME_PROMPT, State, Wrap, evaluate, kind_label, next_deadline,
    parse_meter, wrap_up_prompt,
};
use notify::{EventKind, RecursiveMode, Watcher};
use serde_json::Value;

const DEFAULT_METER: &str = ".claude/plugins/store/sub-meter_better-claude-3857a38a4680.json";
const DEFAULT_STATE: &str = ".local/state/herdr-usage/state.json";
const HELP: &str = "herdr-usage: vigila el uso del plan de Claude (archivo de sub-meter) y actúa sobre los panes de herdr.

Uso: herdr-usage [opciones]

Opciones:
  --meter <ruta>      archivo de sub-meter (por defecto ~/.claude/plugins/store/sub-meter_…json)
  --state <ruta>      archivo de estado (por defecto ~/.local/state/herdr-usage/state.json)
  --threshold <n>     porcentaje que dispara el cierre ordenado (por defecto 90)
  --margin <min>      minutos de espera después del reinicio antes de retomar (por defecto 2)
  --once              evalúa una sola vez y sale
  --status            muestra lecturas, estado y lo que haría, sin actuar
  --dry-run           imprime los comandos de herdr en vez de correrlos
  -h, --help          muestra esta ayuda";

struct Config {
    meter: PathBuf,
    state: PathBuf,
    threshold: f64,
    margin: Duration,
    once: bool,
    status: bool,
    dry_run: bool,
    herdr: String,
}

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"))
}

fn parse_args() -> Result<Option<Config>, String> {
    let mut config = Config {
        meter: home().join(DEFAULT_METER),
        state: home().join(DEFAULT_STATE),
        threshold: 90.0,
        margin: Duration::minutes(2),
        once: false,
        status: false,
        dry_run: false,
        herdr: env::var("HERDR_BIN").unwrap_or_else(|_| "herdr".into()),
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("falta el valor de {name}"));
        match arg.as_str() {
            "--meter" => config.meter = value("--meter")?.into(),
            "--state" => config.state = value("--state")?.into(),
            "--threshold" => {
                config.threshold = value("--threshold")?
                    .parse()
                    .map_err(|_| "--threshold espera un número".to_string())?
            }
            "--margin" => {
                let minutes: i64 = value("--margin")?
                    .parse()
                    .map_err(|_| "--margin espera minutos enteros".to_string())?;
                config.margin = Duration::minutes(minutes);
            }
            "--once" => config.once = true,
            "--status" => config.status = true,
            "--dry-run" => config.dry_run = true,
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("opción desconocida: {other}")),
        }
    }
    Ok(Some(config))
}

fn log(message: &str) {
    eprintln!("{} {message}", Local::now().format("%Y-%m-%d %H:%M:%S"));
}

fn load_state(path: &Path) -> State {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_state(path: &Path, state: &State) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
    fs::rename(tmp, path)
}

fn read_limits(path: &Path) -> Result<Vec<Limit>, String> {
    let raw = fs::read_to_string(path).map_err(|err| format!("no pude leer {}: {err}", path.display()))?;
    parse_meter(&raw).map_err(|err| format!("el archivo de sub-meter no tiene el formato esperado: {err}"))
}

struct Herdr<'a> {
    bin: &'a str,
    dry_run: bool,
}

impl Herdr<'_> {
    fn query(&self, args: &[&str]) -> Result<Value, String> {
        let output = Command::new(self.bin)
            .args(args)
            .output()
            .map_err(|err| format!("no pude correr {}: {err}", self.bin))?;
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| format!("herdr {} no devolvió JSON", args.join(" ")))?;
        match value.get("error") {
            Some(error) => Err(format!("herdr {}: {error}", args.join(" "))),
            None => Ok(value),
        }
    }

    fn act(&self, args: &[&str]) -> Result<(), String> {
        if self.dry_run {
            log(&format!("[dry-run] {} {}", self.bin, args.join(" ")));
            return Ok(());
        }
        self.query(args).map(|_| ())
    }

    fn working_claude_panes(&self) -> Result<Vec<PaneRef>, String> {
        let list = self.query(&["agent", "list"])?;
        let agents = list["result"]["agents"].as_array().cloned().unwrap_or_default();
        Ok(agents
            .iter()
            .filter(|agent| agent["agent"] == "claude" && agent["agent_status"] == "working")
            .filter_map(|agent| {
                Some(PaneRef {
                    pane_id: agent["pane_id"].as_str()?.to_string(),
                    session: agent["agent_session"]["value"].as_str().map(String::from),
                })
            })
            .collect())
    }

    fn ready_for_resume(&self, pane: &PaneRef) -> bool {
        let Ok(info) = self.query(&["agent", "get", &pane.pane_id]) else {
            return false;
        };
        let agent = &info["result"]["agent"];
        let same_session = pane.session.as_deref() == agent["agent_session"]["value"].as_str();
        let status = agent["agent_status"].as_str().unwrap_or_default();
        same_session && matches!(status, "idle" | "done")
    }

    fn prompt(&self, pane: &str, text: &str) -> Result<(), String> {
        self.act(&["agent", "prompt", pane, text])
    }

    fn notify(&self, title: &str, body: &str) {
        let result = self.act(&["notification", "show", title, "--body", body, "--sound", "request"]);
        if let Err(err) = result {
            log(&format!("no se pudo mostrar el aviso: {err}"));
        }
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn kinds_text(kinds: &[String]) -> String {
    kinds.iter().map(|kind| kind_label(kind)).collect::<Vec<_>>().join(" y ")
}

fn when_text(moment: chrono::DateTime<Utc>) -> String {
    let local = moment.with_timezone(&Local);
    if local.date_naive() == Local::now().date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format("%H:%M del %d/%m").to_string()
    }
}

fn wrap_up(herdr: &Herdr, limits: &[Limit], kinds: Vec<String>, resume_at: chrono::DateTime<Utc>) -> Wrap {
    let text = wrap_up_prompt(limits, &kinds);
    let panes = herdr.working_claude_panes().unwrap_or_else(|err| {
        log(&format!("no pude listar los agentes: {err}"));
        Vec::new()
    });
    let mut reached = Vec::new();
    for pane in panes {
        match herdr.prompt(&pane.pane_id, &text) {
            Ok(()) => reached.push(pane),
            Err(err) => log(&format!("no pude avisar a {}: {err}", pane.pane_id)),
        }
    }
    let when = when_text(resume_at);
    let body = if reached.is_empty() {
        format!("Ningún pane estaba trabajando. Se reinicia a las {when}.")
    } else {
        format!(
            "Pedí cierre ordenado a {} pane{}. Los retomo a las {when}.",
            reached.len(),
            plural(reached.len())
        )
    };
    herdr.notify(&format!("Uso {} cerca del límite", kinds_text(&kinds)), &body);
    log(&body);
    Wrap { resume_at, kinds, panes: reached }
}

fn resume(herdr: &Herdr, wrap: &Wrap) {
    let resumed = wrap
        .panes
        .iter()
        .filter(|pane| herdr.ready_for_resume(pane))
        .filter(|pane| match herdr.prompt(&pane.pane_id, RESUME_PROMPT) {
            Ok(()) => true,
            Err(err) => {
                log(&format!("no pude retomar {}: {err}", pane.pane_id));
                false
            }
        })
        .count();
    let body = if wrap.panes.is_empty() {
        "No había panes por retomar.".to_string()
    } else {
        format!("Retomé {resumed} de {} pane{}.", wrap.panes.len(), plural(wrap.panes.len()))
    };
    herdr.notify(&format!("Se reinició el uso {}", kinds_text(&wrap.kinds)), &body);
    log(&body);
}

fn tick(config: &Config, herdr: &Herdr, mut state: State) -> State {
    let limits = match read_limits(&config.meter) {
        Ok(limits) => limits,
        Err(err) => {
            log(&err);
            Vec::new()
        }
    };
    let action = evaluate(&limits, &state, Utc::now(), config.threshold, config.margin);
    if config.status {
        for limit in &limits {
            println!(
                "{}: {:.0} %, se reinicia {}",
                kind_label(&limit.kind),
                limit.percent_used,
                limit.resets_at.with_timezone(&Local).format("%Y-%m-%d %H:%M")
            );
        }
        println!("estado: {}", serde_json::to_string(&state).unwrap_or_default());
        println!("haría: {action:?}");
        return state;
    }
    match action {
        Action::Nothing => return state,
        Action::WrapUp { resume_at, kinds } => {
            state.wrap = Some(wrap_up(herdr, &limits, kinds, resume_at));
        }
        Action::Extend { resume_at, kinds } => {
            if let Some(wrap) = state.wrap.as_mut() {
                wrap.resume_at = resume_at;
                wrap.kinds = kinds;
                log(&format!("el reinicio se recorre a {}", resume_at.with_timezone(&Local)));
            }
        }
        Action::Resume => {
            if let Some(wrap) = state.wrap.take() {
                resume(herdr, &wrap);
            }
        }
    }
    if config.dry_run {
        log(&format!("[dry-run] estado nuevo: {}", serde_json::to_string(&state).unwrap_or_default()));
    } else if let Err(err) = save_state(&config.state, &state) {
        log(&format!("no pude guardar el estado: {err}"));
    }
    state
}

fn watch(config: &Config, herdr: &Herdr) -> Result<(), String> {
    let dir = config.meter.parent().ok_or("la ruta de sub-meter no tiene carpeta")?;
    let name = config.meter.file_name().map(|name| name.to_os_string());
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event {
            let relevant = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
                && event.paths.iter().any(|path| path.file_name().map(|n| n.to_os_string()) == name);
            if relevant {
                let _ = sender.send(());
            }
        }
    })
    .map_err(|err| format!("no pude crear el vigilante de archivos: {err}"))?;
    watcher
        .watch(dir, RecursiveMode::NonRecursive)
        .map_err(|err| format!("no pude vigilar {}: {err}", dir.display()))?;
    log(&format!("vigilando {} (umbral {:.0} %)", config.meter.display(), config.threshold));

    let mut state = tick(config, herdr, load_state(&config.state));
    loop {
        let received = match next_deadline(&state) {
            Some(deadline) => {
                let wait = (deadline - Utc::now()).to_std().unwrap_or(StdDuration::ZERO);
                match receiver.recv_timeout(wait) {
                    Ok(()) => true,
                    Err(mpsc::RecvTimeoutError::Timeout) => false,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Err("se cerró el vigilante".into()),
                }
            }
            None => receiver.recv().map(|_| true).map_err(|_| "se cerró el vigilante".to_string())?,
        };
        if received {
            std::thread::sleep(StdDuration::from_millis(300));
            while receiver.try_recv().is_ok() {}
        }
        state = tick(config, herdr, state);
    }
}

fn main() -> ExitCode {
    let config = match parse_args() {
        Ok(Some(config)) => config,
        Ok(None) => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(err) => {
            eprintln!("{err}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    let herdr = Herdr { bin: &config.herdr, dry_run: config.dry_run };
    if config.once || config.status {
        tick(&config, &herdr, load_state(&config.state));
        return ExitCode::SUCCESS;
    }
    match watch(&config, &herdr) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log(&err);
            ExitCode::FAILURE
        }
    }
}
