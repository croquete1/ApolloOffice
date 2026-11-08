use std::path::Path;

#[allow(dead_code)]
fn xq(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[allow(dead_code)]
fn sq(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    )
}

fn up(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(400),
    )
    .is_ok()
}

fn sfx(dd: &Path) -> String {
    use sha2::{Digest, Sha256};
    if dd == crate::app::dd0() {
        return String::new();
    }
    format!(
        "-{}",
        &hex::encode(Sha256::digest(dd.display().to_string().as_bytes()))[..8]
    )
}

#[allow(dead_code)]
fn nm(dd: &Path) -> (String, String, String) {
    let x = sfx(dd);
    (
        format!("ApolloOffice{x}"),
        format!("dev.apollonet.office{}", x.replace('-', ".")),
        format!("apollo-office{x}"),
    )
}

pub fn autostart(on: bool, me: &Path, dd: &Path, port: u16) -> Result<String, String> {
    let exe = me.display().to_string();
    let (wn, ml, sn) = nm(dd);
    let _ = (&wn, &ml, &sn);
    let mut av: Vec<String> = vec!["--data".into(), dd.display().to_string(), "--quiet".into()];
    if port != 47300 {
        av.extend(["--port".into(), port.to_string()]);
    }
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        let k = winreg::RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(
                r"Software\Microsoft\Windows\CurrentVersion\Run",
                winreg::enums::KEY_SET_VALUE,
            )
            .map_err(|e| e.to_string())?;
        if on {
            k.set_value(
                &wn,
                &format!(
                    "\"{exe}\" {}",
                    av.iter()
                        .map(|x| if x.starts_with("--") {
                            x.clone()
                        } else {
                            format!("\"{x}\"")
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            )
            .map_err(|e| e.to_string())?;
            if !up(port) {
                use std::os::windows::process::CommandExt;
                let _ = std::process::Command::new(&exe)
                    .args(&av)
                    .creation_flags(0x08000008)
                    .spawn();
                return Ok("Started, and starts at login from now on.".into());
            }
            return Ok("Starts at login from now on.".into());
        }
        let _ = k.delete_value(&wn);
        return Ok("Autostart off.".into());
    }
    #[cfg(target_os = "macos")]
    {
        let p = dirs::home_dir()
            .ok_or("no home folder")?
            .join(format!("Library/LaunchAgents/{ml}.plist"));
        if on {
            let x = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array><string>{}</string>{}</array><key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict><key>StandardErrorPath</key><string>{}</string></dict></plist>\n",
                xq(&ml),
                xq(&exe),
                av.iter().map(|x| format!("<string>{}</string>", xq(x))).collect::<String>(),
                xq(&dd.join("apollo-office.log").display().to_string())
            );
            std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&p, x).map_err(|e| e.to_string())?;
            if !up(port) {
                let u = unsafe { libc::getuid() };
                let _ = std::process::Command::new("launchctl")
                    .args(["bootout", &format!("gui/{u}/{ml}")])
                    .output();
                let r = std::process::Command::new("launchctl")
                    .args(["bootstrap", &format!("gui/{u}")])
                    .arg(&p)
                    .output()
                    .map_err(|e| e.to_string())?;
                if !r.status.success() {
                    return Err(format!(
                        "launchctl bootstrap failed: {}",
                        String::from_utf8_lossy(&r.stderr).trim()
                    ));
                }
                return Ok(format!(
                    "Started, and starts at login from now on ({}).",
                    p.display()
                ));
            }
            return Ok(format!("Starts at login from now on ({}).", p.display()));
        }
        let _ = std::fs::remove_file(&p);
        return Ok("Autostart off.".into());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let d = dirs::config_dir()
            .ok_or("no config folder")?
            .join("systemd/user");
        let p = d.join(format!("{sn}.service"));
        if on {
            std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            std::fs::write(&p, format!("[Unit]\nDescription=ApolloOffice\n\n[Service]\nExecStart={} {}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n", sq(&exe), av.iter().map(|x| sq(x)).collect::<Vec<_>>().join(" "))).map_err(|e| e.to_string())?;
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            let now = !up(port);
            let _ = std::process::Command::new("systemctl")
                .args(if now {
                    vec!["--user", "enable", "--now", &sn]
                } else {
                    vec!["--user", "enable", &sn]
                })
                .status();
            return Ok(if now {
                "Started, and starts at login from now on (systemd --user).".into()
            } else {
                "Starts at login from now on (systemd --user).".into()
            });
        }
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", &sn])
            .status();
        let _ = std::fs::remove_file(&p);
        Ok("Autostart off.".into())
    }
}

pub fn autostart_on(dd: &Path) -> bool {
    let (wn, ml, sn) = nm(dd);
    let _ = (&wn, &ml, &sn);
    #[cfg(windows)]
    {
        return winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
            .and_then(|k| k.get_value::<String, _>(&wn))
            .is_ok();
    }
    #[cfg(target_os = "macos")]
    {
        return dirs::home_dir()
            .map(|h| h.join(format!("Library/LaunchAgents/{ml}.plist")).exists())
            .unwrap_or(false);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        dirs::config_dir()
            .map(|h| h.join(format!("systemd/user/{sn}.service")).exists())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod t {
    #[test]
    fn names() {
        let d = crate::app::dd0();
        assert_eq!(super::nm(&d).1, "dev.apollonet.office");
        let (w, m, s) = super::nm(std::path::Path::new("/tmp/ao2"));
        assert!(
            w.starts_with("ApolloOffice-")
                && m.starts_with("dev.apollonet.office.")
                && s.starts_with("apollo-office-")
                && m.len() == 29
        );
    }
}
