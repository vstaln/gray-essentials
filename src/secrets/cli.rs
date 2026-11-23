use std::io::{BufRead, IsTerminal, Read, Write};

use super::mask::MIN_LEN;
use super::{store, util};

pub const USAGE: &str = "usage: gray secrets <command>
  set NAME                                   store a secret (typed without echo)
  rm NAME                                    delete a stored secret
  list                                       names, lifetime, created, last used
  exec --env NAME[,NAME] [--file NAME] -- CMD  run CMD with secrets injected, output masked
  elevate-setup                              print the lines that enable gray-askpass";

fn one_name(args: &[String]) -> Option<&str> {
    match args {
        [name] if util::valid_name(name) => Some(name),
        _ => {
            eprintln!("expected one secret name (letters, digits, underscores)\n{USAGE}");
            None
        }
    }
}

fn read_hidden(prompt: &str) -> std::io::Result<String> {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        let mut s = String::new();
        stdin.lock().read_to_string(&mut s)?;
        return Ok(s.trim_end_matches(['\r', '\n']).to_string());
    }
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    let fd = libc::STDIN_FILENO;
    let have = unsafe { libc::tcgetattr(fd, &mut saved) } == 0;
    if have {
        let mut quiet = saved;
        quiet.c_lflag &= !libc::ECHO;
        quiet.c_lflag |= libc::ECHONL;
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) };
    }
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    if have {
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
    }
    read?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

pub fn set(args: &[String]) -> i32 {
    let Some(name) = one_name(args) else { return 2 };
    let value = match read_hidden(&format!("Value for {name}: ")) {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => {
            eprintln!("empty value, nothing stored");
            return 1;
        }
        Err(e) => {
            eprintln!("could not read the value: {e}");
            return 1;
        }
    };
    if value.len() < MIN_LEN {
        eprintln!(
            "warning: {name} is shorter than {MIN_LEN} characters, so it will not be masked in output"
        );
    }
    match store::store(name, &value) {
        Ok(()) => {
            util::audit(&util::session_id(), name, "set", "(permanent, cli)");
            println!("stored {name}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

pub fn rm(args: &[String]) -> i32 {
    let Some(name) = one_name(args) else { return 2 };
    match store::clear(name) {
        Ok(()) => {
            util::audit(&util::session_id(), name, "rm", "(cli)");
            println!("removed {name}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

pub fn list() -> i32 {
    println!("{}", store::table(&[]));
    0
}

pub fn elevate_setup() -> i32 {
    let askpass = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.ancestors()
                .skip(1)
                .take(4)
                .map(|d| d.join("gray-askpass"))
                .find(|p| p.is_file())
        })
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/path/to/gray-extras/gray-askpass".into());
    println!("gray-secrets does not edit system files. Add these lines yourself.");
    println!();
    println!("1. Polkit agent: add after the nm-applet line in ~/.config/bspwm/bspwmrc");
    println!("   (needs the xfce-polkit package):");
    println!();
    println!(
        "pgrep -x xfce-polkit >/dev/null || setsid -f /usr/libexec/xfce-polkit >/tmp/xfce-polkit.log 2>&1 &"
    );
    println!();
    println!("2. Sudo password helper: add to /etc/sudo.conf (as root; needs gnome-ssh-askpass");
    println!("   at /usr/bin/gnome-ssh-askpass):");
    println!();
    println!("Path askpass {askpass}");
    println!();
    println!("Gray's bash tool exports SUDO_ASKPASS=/bin/false, which overrides sudo.conf, so an");
    println!("agent must run: SUDO_ASKPASS={askpass} sudo -A CMD");
    0
}
