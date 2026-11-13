use chrono::{DateTime, Datelike, Local, Timelike, Utc};

pub fn job_name(session: &str, at: DateTime<Utc>) -> String {
    format!("herdr-usage-resume-{session}-{}", at.timestamp())
}

pub fn launchd_label(job: &str) -> String {
    format!("dev.herdr-usage.{job}")
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn launchd_plist(label: &str, program: &[String], at: DateTime<Local>) -> String {
    let at = if at.second() > 0 || at.nanosecond() > 0 {
        at.with_second(0).and_then(|value| value.with_nanosecond(0)).unwrap_or(at) + chrono::Duration::minutes(1)
    } else {
        at
    };
    let arguments: String = program
        .iter()
        .map(|arg| format!("\n        <string>{}</string>", xml_escape(arg)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>{arguments}
    </array>
    <key>StartCalendarInterval</key>
    <dict>
        <key>Month</key>
        <integer>{month}</integer>
        <key>Day</key>
        <integer>{day}</integer>
        <key>Hour</key>
        <integer>{hour}</integer>
        <key>Minute</key>
        <integer>{minute}</integer>
    </dict>
</dict>
</plist>
"#,
        label = xml_escape(label),
        month = at.month(),
        day = at.day(),
        hour = at.hour(),
        minute = at.minute(),
    )
}

pub fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(ch);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

pub fn windows_register_script(job: &str, program: &[String], at: DateTime<Local>) -> String {
    let (exe, args) = program.split_first().map_or(("", &[][..]), |(exe, args)| (exe.as_str(), args));
    let arguments = args.iter().map(|arg| windows_arg(arg)).collect::<Vec<_>>().join(" ");
    format!(
        "$ErrorActionPreference = 'Stop'; \
$at = [DateTime]::new({year}, {month}, {day}, {hour}, {minute}, {second}); \
$trigger = New-ScheduledTaskTrigger -Once -At $at; \
$trigger.EndBoundary = $at.AddHours(1).ToString('s'); \
$action = New-ScheduledTaskAction -Execute {exe} -Argument {arguments}; \
$settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -DeleteExpiredTaskAfter (New-TimeSpan -Seconds 1); \
Register-ScheduledTask -TaskPath '\\herdr-usage\\' -TaskName {job} -Trigger $trigger -Action $action -Settings $settings -Force | Out-Null",
        year = at.year(),
        month = at.month(),
        day = at.day(),
        hour = at.hour(),
        minute = at.minute(),
        second = at.second(),
        exe = ps_quote(exe),
        arguments = ps_quote(&arguments),
        job = ps_quote(job),
    )
}

pub fn windows_task(job: &str) -> String {
    format!("\\herdr-usage\\{job}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn local(hour: u32, minute: u32, second: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 8, hour, minute, second).unwrap()
    }

    #[test]
    fn job_name_has_session_and_time() {
        let at = Utc.with_ymd_and_hms(2026, 11, 8, 10, 0, 0).unwrap();
        assert_eq!(job_name("default", at), format!("herdr-usage-resume-default-{}", at.timestamp()));
    }

    #[test]
    fn launchd_rounds_up_to_the_next_minute() {
        let plist = launchd_plist("dev.herdr-usage.x", &["/bin/herdr-usage".into(), "hook".into()], local(10, 2, 30));
        assert!(plist.contains("<key>Hour</key>\n        <integer>10</integer>"));
        assert!(plist.contains("<key>Minute</key>\n        <integer>3</integer>"));
        assert!(plist.contains("<string>/bin/herdr-usage</string>\n        <string>hook</string>"));
        let exact = launchd_plist("x", &[], local(10, 2, 0));
        assert!(exact.contains("<key>Minute</key>\n        <integer>2</integer>"));
    }

    #[test]
    fn launchd_escapes_xml() {
        let plist = launchd_plist("x", &["/tmp/a&b<c>".into()], local(10, 0, 0));
        assert!(plist.contains("<string>/tmp/a&amp;b&lt;c&gt;</string>"));
    }

    #[test]
    fn windows_arguments_survive_spaces_and_quotes() {
        assert_eq!(windows_arg("hook"), "hook");
        assert_eq!(windows_arg(r"C:\Users\Ana Pérez\state.json"), r#""C:\Users\Ana Pérez\state.json""#);
        assert_eq!(windows_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(windows_arg(r"C:\dir with space\"), r#""C:\dir with space\\""#);
        assert_eq!(windows_arg(""), r#""""#);
    }

    #[test]
    fn windows_script_quotes_for_powershell() {
        let program = vec![r"C:\it's\herdr-usage.exe".to_string(), "hook".into(), r"C:\a b".into()];
        let script = windows_register_script("job-1", &program, local(10, 2, 30));
        assert!(script.contains("[DateTime]::new(2026, 11, 8, 10, 2, 30)"));
        assert!(script.contains(r"-Execute 'C:\it''s\herdr-usage.exe'"));
        assert!(script.contains(r#"-Argument 'hook "C:\a b"'"#));
        assert!(script.contains("-TaskName 'job-1'"));
    }
}
