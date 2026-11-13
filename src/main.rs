use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use chrono::{DateTime, Duration, Local, Utc};
use herdr_usage::schedule::job_name;
use herdr_usage::{
    Action, Agent, Limit, PaneRef, RESUME_PROMPT, Readiness, State, Wrap, catch_up_targets, evaluate,
    in_wrap_window, kind_label, limits_from_statusline, merge_limits, meter_json, parse_meter, readiness, render_meter,
    retry_resume, session_from_socket, timer_needed, wrap_up_prompt, wrap_up_targets,
};
use serde_json::Value;
#[cfg(unix)]
use herdr_usage::{LinkAction, LinkState, stable_link_action};

const PLUGIN_STATE_SUBDIR: &str = "herdr/plugins/herdr-usage";
const METER_FILE: &str = "meter/claude.json";
#[cfg(unix)]
const STABLE_LINK: &str = ".local/bin/herdr-usage";
const HELP: &str = "herdr-usage: plugin de herdr que mide el uso del plan de Claude y actúa sobre los panes.

Uso: herdr-usage <hook|status|statusline> [opciones]

Comandos:
  hook                evalúa el uso y actúa; lo llaman los eventos de herdr, el arranque y la tarea de regreso
  status              muestra lecturas, estado y lo que haría, sin actuar
  statusline          statusline de Claude Code: lee el JSON por stdin, guarda el uso y dibuja la barra

Opciones:
  --meter <ruta>      archivo de uso (por defecto <estado del plugin>/meter/claude.json)
  --state <ruta>      archivo de estado (por defecto <estado del plugin>/<sesión>.json)
  --threshold <n>     porcentaje que dispara el cierre ordenado (por defecto 90)
  --margin <min>      minutos de espera después del reinicio antes de retomar (por defecto 2)
  --herdr <ruta>      binario de herdr (por defecto HERDR_BIN_PATH o herdr)
  --socket <ruta>     socket de la sesión de herdr (por defecto HERDR_SOCKET_PATH)
  --job <nombre>      lo pasa la tarea de regreso para borrarse al terminar
  --dry-run           imprime los comandos de herdr y del programador en vez de correrlos
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
    socket: Option<String>,
    job: Option<String>,
}

fn home() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
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
        state: PathBuf::new(),
        threshold: 90.0,
        margin: Duration::minutes(2),
        dry_run: false,
        herdr: env::var("HERDR_BIN_PATH")
            .or_else(|_| env::var("HERDR_BIN"))
            .unwrap_or_else(|_| "herdr".into()),
        session: String::new(),
        event: env::var("HERDR_PLUGIN_EVENT").ok(),
        event_pane: event_pane(),
        only_pane: None,
        socket: env::var("HERDR_SOCKET_PATH").ok(),
        job: None,
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
            "--herdr" => config.herdr = value("--herdr")?,
            "--socket" => config.socket = Some(value("--socket")?),
            "--job" => config.job = Some(value("--job")?),
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("opción desconocida: {other}")),
        }
    }
    config.session = session_from_socket(config.socket.as_deref());
    if config.state.as_os_str().is_empty() {
        config.state = state_dir.join(format!("{}.json", config.session));
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

#[cfg(not(unix))]
fn ensure_stable_link(_config: &Config) {}

#[cfg(unix)]
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
    socket: Option<&'a str>,
    dry_run: bool,
}

impl Herdr<'_> {
    fn query(&self, args: &[&str]) -> Result<Value, String> {
        let mut command = Command::new(self.bin);
        if let Some(socket) = self.socket {
            command.env("HERDR_SOCKET_PATH", socket);
        }
        let output = command
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

    fn readiness(&self, pane: &PaneRef) -> Readiness {
        let Ok(info) = self.query(&["agent", "get", &pane.pane_id]) else {
            return Readiness::Unknown;
        };
        let agent = &info["result"]["agent"];
        readiness(
            pane,
            agent["agent_session"]["value"].as_str(),
            agent["agent_status"].as_str().unwrap_or_default(),
        )
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
    let listed = herdr.agents();
    let targets = match &listed {
        Ok(agents) => wrap_up_targets(agents, config.event_pane.as_deref()),
        Err(err) => {
            log(&format!("no pude listar los agentes: {err}"));
            Vec::new()
        }
    };
    let reached = deliver(config, herdr, targets, &text);
    let when = when_text(resume_at);
    let body = if listed.is_err() {
        format!("No pude ver los panes; reintento en el siguiente evento. Se reinicia a las {when}.")
    } else if reached.is_empty() {
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
    Wrap { resume_at, kinds, panes: reached, attempts: 0 }
}

fn deliver(config: &Config, herdr: &Herdr, targets: Vec<PaneRef>, text: &str) -> Vec<PaneRef> {
    targets
        .into_iter()
        .filter(|pane| config.only_pane.as_ref().is_none_or(|only| &pane.pane_id == only))
        .filter(|pane| match herdr.prompt(&pane.pane_id, text) {
            Ok(()) => true,
            Err(err) => {
                log(&format!("no pude avisar a {}: {err}", pane.pane_id));
                false
            }
        })
        .collect()
}

fn catch_up(config: &Config, herdr: &Herdr, limits: &[Limit], wrap: &mut Wrap) {
    let agents = match herdr.agents() {
        Ok(agents) => agents,
        Err(err) => {
            log(&format!("no pude listar los agentes: {err}"));
            return;
        }
    };
    let targets = catch_up_targets(&agents, config.event_pane.as_deref(), wrap);
    if targets.is_empty() {
        return;
    }
    let reached = deliver(config, herdr, targets, &wrap_up_prompt(limits, &wrap.kinds));
    if !reached.is_empty() {
        log(&format!("pedí cierre ordenado a {} pane{} más", reached.len(), plural(reached.len())));
        wrap.panes.extend(reached);
    }
}

fn resume(herdr: &Herdr, wrap: &Wrap) -> Option<Wrap> {
    let mut resumed = 0;
    let mut failed = Vec::new();
    for pane in &wrap.panes {
        match herdr.readiness(pane) {
            Readiness::Skip => {}
            Readiness::Unknown => failed.push(pane.clone()),
            Readiness::Ready => match herdr.prompt(&pane.pane_id, RESUME_PROMPT) {
                Ok(()) => resumed += 1,
                Err(err) => {
                    log(&format!("no pude retomar {}: {err}", pane.pane_id));
                    failed.push(pane.clone());
                }
            },
        }
    }
    let retry = retry_resume(wrap, failed, Utc::now());
    let total = wrap.panes.len();
    let mut body = if total == 0 {
        "No había panes por retomar.".to_string()
    } else {
        format!("Retomé {resumed} de {total} pane{}.", plural(total))
    };
    if let Some(next) = &retry {
        body.push_str(&format!(" Reintento a las {}.", when_text(next.resume_at)));
    }
    let first = wrap.attempts == 0;
    if first || retry.is_none() {
        let title = if first { "Se reinició el uso" } else { "Reintento de regreso, uso" };
        herdr.notify(&format!("{title} {}", kinds_text(&wrap.kinds)), &body);
    }
    log(&body);
    retry
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

fn succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn job_program(config: &Config, job: &str) -> Result<Vec<String>, String> {
    let exe = env::current_exe().map_err(|err| format!("no sé dónde está el binario: {err}"))?;
    let mut program: Vec<String> = vec![
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
        "--herdr".into(),
        config.herdr.clone(),
    ];
    if let Some(socket) = &config.socket {
        program.extend(["--socket".into(), socket.clone()]);
    }
    program.extend(["--job".into(), job.to_string()]);
    Ok(program)
}

#[cfg(target_os = "linux")]
mod scheduler {
    use super::*;

    pub fn arm(config: &Config, job: &str, at: DateTime<Utc>) -> Result<(), String> {
        let mut args: Vec<String> = vec![
            "--user".into(),
            "--collect".into(),
            "--quiet".into(),
            "--unit".into(),
            job.into(),
            "--on-calendar".into(),
            at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            "--timer-property=AccuracySec=1s".into(),
        ];
        args.extend(job_program(config, job)?);
        run_or_print(config, "systemd-run", &args)
    }

    pub fn cancel(config: &Config, job: &str) {
        let _ = run_or_print(config, "systemctl", &["--user".into(), "stop".into(), format!("{job}.timer")]);
    }

    pub fn exists(job: &str) -> bool {
        succeeds("systemctl", &["--user", "is-active", "--quiet", &format!("{job}.timer")])
    }

    pub fn finish(_config: &Config, _job: &str) {}

    pub fn describe(job: &str) -> String {
        format!("{job}.timer (systemd de usuario)")
    }
}

#[cfg(target_os = "macos")]
mod scheduler {
    use super::*;
    use herdr_usage::schedule::{launchd_label, launchd_plist};

    fn domain() -> String {
        let uid = Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default();
        format!("gui/{uid}")
    }

    fn plist_path(label: &str) -> PathBuf {
        home().join("Library/LaunchAgents").join(format!("{label}.plist"))
    }

    pub fn arm(config: &Config, job: &str, at: DateTime<Utc>) -> Result<(), String> {
        let label = launchd_label(job);
        let path = plist_path(&label);
        let plist = launchd_plist(&label, &job_program(config, job)?, at.with_timezone(&Local));
        if config.dry_run {
            log(&format!("[dry-run] escribiría {}:\n{plist}", path.display()));
        } else {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(|err| format!("no pude crear {}: {err}", dir.display()))?;
            }
            fs::write(&path, plist).map_err(|err| format!("no pude escribir {}: {err}", path.display()))?;
        }
        run_or_print(config, "launchctl", &["bootstrap".into(), domain(), path.display().to_string()])
    }

    pub fn cancel(config: &Config, job: &str) {
        let label = launchd_label(job);
        if !config.dry_run {
            let _ = fs::remove_file(plist_path(&label));
        }
        let _ = run_or_print(config, "launchctl", &["bootout".into(), format!("{}/{label}", domain())]);
    }

    pub fn exists(job: &str) -> bool {
        succeeds("launchctl", &["print", &format!("{}/{}", domain(), launchd_label(job))])
    }

    pub fn finish(config: &Config, job: &str) {
        cancel(config, job);
    }

    pub fn describe(job: &str) -> String {
        format!("{} (launchd)", launchd_label(job))
    }
}

#[cfg(windows)]
mod scheduler {
    use super::*;
    use herdr_usage::schedule::{windows_register_script, windows_task};

    pub fn arm(config: &Config, job: &str, at: DateTime<Utc>) -> Result<(), String> {
        let script = windows_register_script(job, &job_program(config, job)?, at.with_timezone(&Local));
        run_or_print(
            config,
            "powershell",
            &["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script],
        )
    }

    pub fn cancel(config: &Config, job: &str) {
        let _ = run_or_print(config, "schtasks", &["/Delete".into(), "/TN".into(), windows_task(job), "/F".into()]);
    }

    pub fn exists(job: &str) -> bool {
        succeeds("schtasks", &["/Query", "/TN", &windows_task(job)])
    }

    pub fn finish(config: &Config, job: &str) {
        cancel(config, job);
    }

    pub fn describe(job: &str) -> String {
        format!("{} (Programador de tareas)", windows_task(job))
    }
}

fn sync_timer(config: &Config, state: &mut State) {
    let Some(wrap) = state.wrap.as_ref() else {
        state.timer_for = None;
        return;
    };
    if config.event.as_deref() == Some("startup") && !scheduler::exists(&job_name(&config.session, wrap.resume_at)) {
        state.timer_for = None;
    }
    let Some(at) = timer_needed(state) else {
        return;
    };
    if let Some(previous) = state.timer_for {
        let previous = job_name(&config.session, previous);
        if config.job.as_deref() != Some(previous.as_str()) {
            scheduler::cancel(config, &previous);
        }
    }
    match scheduler::arm(config, &job_name(&config.session, at), at) {
        Ok(()) => {
            state.timer_for = Some(at);
            log(&format!("tarea de regreso programada para {}", when_text(at)));
        }
        Err(err) => log(&format!("no pude programar la tarea de regreso: {err}")),
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
        let job = job_name(&config.session, at);
        let active = if scheduler::exists(&job) { "programada" } else { "no está programada" };
        println!("tarea de regreso: {}, {active}", scheduler::describe(&job));
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
        Action::Nothing => {
            if in_wrap_window(&state, Utc::now())
                && let Some(wrap) = state.wrap.as_mut()
            {
                catch_up(config, herdr, &limits, wrap);
            }
        }
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
                state.wrap = resume(herdr, &wrap);
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
    let herdr = Herdr {
        bin: &config.herdr,
        socket: config.socket.as_deref(),
        dry_run: config.dry_run,
    };
    let result = hook(&config, &herdr);
    if let Some(job) = &config.job {
        scheduler::finish(&config, job);
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log(&err);
            ExitCode::FAILURE
        }
    }
}
