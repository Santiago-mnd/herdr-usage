use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use chrono::{DateTime, Duration, Local, Utc};
use herdr_usage::{
    Action, Agent, Limit, LinkAction, LinkState, PaneRef, RESUME_PROMPT, State, Wrap, evaluate, kind_label, limits_from_statusline,
    merge_limits, meter_json, parse_meter, render_meter, session_from_socket, stable_link_action, timer_needed,
    wrap_up_prompt,
    wrap_up_targets,
};
use serde_json::Value;

const PLUGIN_STATE_SUBDIR: &str = "herdr/plugins/herdr-usage";
const METER_FILE: &str = "meter/claude.json";
const STABLE_LINK: &str = ".local/bin/herdr-usage";
const TIMER_ENV: [&str; 4] = [
    "HERDR_BIN_PATH",
    "HERDR_SOCKET_PATH",
    "HERDR_PLUGIN_STATE_DIR",
    "HERDR_PLUGIN_ID",
];
const HELP: &str = "herdr-usage: plugin de herdr que mide el uso del plan de Claude y actúa sobre los panes.

Uso: herdr-usage <hook|status|statusline> [opciones]

Comandos:
  hook                evalúa el uso y actúa; lo llaman los eventos de herdr, el arranque y el timer de regreso
  status              muestra lecturas, estado y lo que haría, sin actuar
  statusline          statusline de Claude Code: lee el JSON por stdin, guarda el uso y dibuja la barra

Opciones:
  --meter <ruta>      archivo de uso (por defecto <estado del plugin>/meter/claude.json)
  --state <ruta>      archivo de estado (por defecto <estado del plugin>/<sesión>.json)
  --threshold <n>     porcentaje que dispara el cierre ordenado (por defecto 90)
  --margin <min>      minutos de espera después del reinicio antes de retomar (por defecto 2)
  --dry-run           imprime los comandos de herdr y de systemd en vez de correrlos
  --only-pane <id>    para pruebas: el cierre solo se le pide a ese pane
  -h, --help          muestra esta ayuda";

#[derive(PartialEq)]
enum Mode {
    Hook,
    Status,
    Statusline,
}

struct Config {
    mode: Mode,
    meter: PathBuf,
    state: PathBuf,
    threshold: f64,
    margin: Duration,
    dry_run: bool,
    herdr: String,
    session: String,
    event: Option<String>,
    event_pane: Option<String>,
    only_pane: Option<String>,
}

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"))
}

fn plugin_state_dir() -> PathBuf {
    env::var_os("HERDR_PLUGIN_STATE_DIR").map(PathBuf::from).unwrap_or_else(|| {
        env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/state"))
            .join(PLUGIN_STATE_SUBDIR)
    })
}

fn event_pane() -> Option<String> {
    let raw = env::var("HERDR_PLUGIN_EVENT_JSON").ok()?;
    let event: Value = serde_json::from_str(&raw).ok()?;
    event["data"]["pane_id"].as_str().map(String::from)
}

fn parse_args() -> Result<Option<Config>, String> {
    let session = session_from_socket(env::var("HERDR_SOCKET_PATH").ok().as_deref());
    let state_dir = plugin_state_dir();
    let mut args = env::args().skip(1);
    let mode = match args.next().as_deref() {
        Some("hook") => Mode::Hook,
        Some("status") => Mode::Status,
        Some("statusline") => Mode::Statusline,
        Some("-h" | "--help") | None => return Ok(None),
        Some(other) => return Err(format!("comando desconocido: {other}")),
    };
    let mut config = Config {
        mode,
        meter: state_dir.join(METER_FILE),
        state: state_dir.join(format!("{session}.json")),
        threshold: 90.0,
        margin: Duration::minutes(2),
        dry_run: false,
        herdr: env::var("HERDR_BIN_PATH")
            .or_else(|_| env::var("HERDR_BIN"))
            .unwrap_or_else(|_| "herdr".into()),
        session,
        event: env::var("HERDR_PLUGIN_EVENT").ok(),
        event_pane: event_pane(),
        only_pane: None,
    };
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
            "--dry-run" => config.dry_run = true,
            "--only-pane" => config.only_pane = Some(value("--only-pane")?),
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("opción desconocida: {other}")),
        }
    }
    Ok(Some(config))
}

fn log(message: &str) {
    eprintln!("{} {message}", Local::now().format("%Y-%m-%d %H:%M:%S"));
}

fn lock_state(path: &Path) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))?;
    file.lock()?;
    Ok(file)
}

fn load_state(path: &Path) -> State {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_state(path: &Path, state: &State) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
    fs::rename(tmp, path)
}

fn read_limits(path: &Path) -> Result<Vec<Limit>, String> {
    let raw = fs::read_to_string(path).map_err(|err| format!("no pude leer {}: {err}", path.display()))?;
    parse_meter(&raw).map_err(|err| format!("el archivo de uso no tiene el formato esperado: {err}"))
}

fn store_fresh_limits(path: &Path, fresh: &[Limit]) -> std::io::Result<Vec<Limit>> {
    let _lock = lock_state(path)?;
    let stored = read_limits(path).unwrap_or_default();
    let merged = merge_limits(&stored, fresh);
    if merged != stored {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, meter_json(&merged))?;
        fs::rename(tmp, path)?;
    }
    Ok(merged)
}

fn ensure_stable_link(config: &Config) {
    if config.dry_run || env::var_os("HERDR_PLUGIN_ID").is_none() {
        return;
    }
    let Ok(exe) = env::current_exe().and_then(fs::canonicalize) else {
        return;
    };
    let link = home().join(STABLE_LINK);
    let target = fs::read_link(&link).ok();
    let current = match (fs::symlink_metadata(&link), target.as_deref()) {
        (Err(_), _) => LinkState::Missing,
        (Ok(_), Some(target)) => LinkState::Symlink(target),
        (Ok(_), None) => LinkState::OtherFile,
    };
    let result = match stable_link_action(current, &exe) {
        LinkAction::Keep => return,
        LinkAction::Blocked => {
            log(&format!("{} ya existe y no es un enlace; no lo toco", link.display()));
            return;
        }
        LinkAction::Replace => fs::remove_file(&link).and_then(|_| std::os::unix::fs::symlink(&exe, &link)),
        LinkAction::Create => link
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| std::os::unix::fs::symlink(&exe, &link)),
    };
    match result {
        Ok(()) => log(&format!("enlace {} → {}", link.display(), exe.display())),
        Err(err) => log(&format!("no pude crear el enlace {}: {err}", link.display())),
    }
}

fn statusline(config: &Config) {
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let fresh = limits_from_statusline(&raw).unwrap_or_default();
    let limits = store_fresh_limits(&config.meter, &fresh)
        .unwrap_or_else(|_| read_limits(&config.meter).unwrap_or(fresh));
    let columns = std::env::var("COLUMNS").ok().and_then(|value| value.parse().ok());
    println!("{}", render_meter(&limits, Utc::now(), columns));
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

    fn agents(&self) -> Result<Vec<Agent>, String> {
        let list = self.query(&["agent", "list"])?;
        let agents = list["result"]["agents"].as_array().cloned().unwrap_or_default();
        Ok(agents
            .iter()
            .filter_map(|agent| {
                Some(Agent {
                    pane_id: agent["pane_id"].as_str()?.to_string(),
                    agent: agent["agent"].as_str().unwrap_or_default().to_string(),
                    status: agent["agent_status"].as_str().unwrap_or_default().to_string(),
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

fn when_text(moment: DateTime<Utc>) -> String {
    let local = moment.with_timezone(&Local);
    if local.date_naive() == Local::now().date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format("%H:%M del %d/%m").to_string()
    }
}

fn wrap_up(config: &Config, herdr: &Herdr, limits: &[Limit], kinds: Vec<String>, resume_at: DateTime<Utc>) -> Wrap {
    let text = wrap_up_prompt(limits, &kinds);
    let agents = herdr.agents().unwrap_or_else(|err| {
        log(&format!("no pude listar los agentes: {err}"));
        Vec::new()
    });
    let mut reached = Vec::new();
    let targets = wrap_up_targets(&agents, config.event_pane.as_deref())
        .into_iter()
        .filter(|pane| config.only_pane.as_ref().is_none_or(|only| &pane.pane_id == only));
    for pane in targets {
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

fn timer_unit(session: &str, at: DateTime<Utc>) -> String {
    format!("herdr-usage-resume-{session}-{}", at.timestamp())
}

fn timer_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", &format!("{unit}.timer")])
        .status()
        .is_ok_and(|status| status.success())
}

fn run_or_print(config: &Config, program: &str, args: &[String]) -> Result<(), String> {
    if config.dry_run {
        log(&format!("[dry-run] {program} {}", args.join(" ")));
        return Ok(());
    }
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("no pude correr {program}: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn cancel_timer(config: &Config, at: DateTime<Utc>) {
    let unit = format!("{}.timer", timer_unit(&config.session, at));
    let _ = run_or_print(config, "systemctl", &["--user".into(), "stop".into(), unit]);
}

fn arm_timer(config: &Config, at: DateTime<Utc>) -> Result<(), String> {
    let exe = env::current_exe().map_err(|err| format!("no sé dónde está el binario: {err}"))?;
    let mut args: Vec<String> = vec![
        "--user".into(),
        "--collect".into(),
        "--quiet".into(),
        "--unit".into(),
        timer_unit(&config.session, at),
        "--on-calendar".into(),
        at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        "--timer-property=AccuracySec=1s".into(),
    ];
    args.extend(
        TIMER_ENV
            .iter()
            .filter_map(|key| env::var(key).ok().map(|value| format!("--setenv={key}={value}"))),
    );
    args.extend([
        exe.display().to_string(),
        "hook".into(),
        "--meter".into(),
        config.meter.display().to_string(),
        "--state".into(),
        config.state.display().to_string(),
        "--threshold".into(),
        config.threshold.to_string(),
        "--margin".into(),
        config.margin.num_minutes().to_string(),
    ]);
    run_or_print(config, "systemd-run", &args)
}

fn sync_timer(config: &Config, state: &mut State) {
    let Some(wrap) = state.wrap.as_ref() else {
        state.timer_for = None;
        return;
    };
    if config.event.as_deref() == Some("startup") && !timer_active(&timer_unit(&config.session, wrap.resume_at)) {
        state.timer_for = None;
    }
    let Some(at) = timer_needed(state) else {
        return;
    };
    if let Some(previous) = state.timer_for {
        cancel_timer(config, previous);
    }
    match arm_timer(config, at) {
        Ok(()) => {
            state.timer_for = Some(at);
            log(&format!("timer de regreso armado para {}", when_text(at)));
        }
        Err(err) => log(&format!("no pude armar el timer de regreso: {err}")),
    }
}

fn status(config: &Config, limits: &[Limit], state: &State, action: &Action) {
    for limit in limits {
        println!(
            "{}: {:.0} %, se reinicia {}",
            kind_label(&limit.kind),
            limit.percent_used,
            limit.resets_at.with_timezone(&Local).format("%Y-%m-%d %H:%M")
        );
    }
    println!("sesión: {}", config.session);
    println!("estado ({}): {}", config.state.display(), serde_json::to_string(state).unwrap_or_default());
    if let Some(at) = state.timer_for {
        let unit = timer_unit(&config.session, at);
        let active = if timer_active(&unit) { "activo" } else { "no está activo" };
        println!("timer: {unit}.timer, {active}");
    }
    println!("haría: {action:?}");
}

fn hook(config: &Config, herdr: &Herdr) -> Result<(), String> {
    let _lock = lock_state(&config.state).map_err(|err| format!("no pude tomar el candado del estado: {err}"))?;
    let before = load_state(&config.state);
    let limits = read_limits(&config.meter).unwrap_or_else(|err| {
        log(&err);
        Vec::new()
    });
    let action = evaluate(&limits, &before, Utc::now(), config.threshold, config.margin);
    if config.mode == Mode::Status {
        status(config, &limits, &before, &action);
        return Ok(());
    }
    let mut state = before.clone();
    match action {
        Action::Nothing => {}
        Action::WrapUp { resume_at, kinds } => {
            state.wrap = Some(wrap_up(config, herdr, &limits, kinds, resume_at));
        }
        Action::Extend { resume_at, kinds } => {
            if let Some(wrap) = state.wrap.as_mut() {
                wrap.resume_at = resume_at;
                wrap.kinds = kinds;
                log(&format!("el reinicio se recorre a {}", when_text(resume_at)));
            }
        }
        Action::Resume => {
            if let Some(wrap) = state.wrap.take() {
                resume(herdr, &wrap);
            }
        }
    }
    sync_timer(config, &mut state);
    if state == before {
        return Ok(());
    }
    if config.dry_run {
        log(&format!("[dry-run] estado nuevo: {}", serde_json::to_string(&state).unwrap_or_default()));
        return Ok(());
    }
    save_state(&config.state, &state).map_err(|err| format!("no pude guardar el estado: {err}"))
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
    if config.mode == Mode::Statusline {
        statusline(&config);
        return ExitCode::SUCCESS;
    }
    ensure_stable_link(&config);
    let herdr = Herdr { bin: &config.herdr, dry_run: config.dry_run };
    match hook(&config, &herdr) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log(&err);
            ExitCode::FAILURE
        }
    }
}
