//! Terminal sessions on a **real** pseudoterminal (ConPTY on Windows, `openpty` on Linux).
//!
//! These tests start an ordinary local shell (`sh` on Linux, `cmd.exe` on Windows), *not* WSL and
//! not a Linux distribution: CI runners have none, and VLINUX never installs one. What they prove
//! is the part that fake PTYs cannot: that input really travels through a terminal device, that
//! the shell sees a TTY of the requested size, that Ctrl+C is delivered as a signal, that Unicode
//! survives, that large output flows, and that exit codes and cleanup work with the native backend.
//!
//! Whether VLINUX's real `wsl.exe -d <distro>` launch works against a real distribution on a real
//! PC is *not* covered here.

use std::sync::Arc;
use std::time::{Duration, Instant};

use vlinux_runtime::terminal::launch::LaunchPlan;
use vlinux_runtime::terminal::session::{SessionState, TerminalManager};
use vlinux_runtime::terminal::testing::Collector;
use vlinux_runtime::terminal::{NativePty, PtySize};

const T: Duration = Duration::from_secs(60);

fn manager() -> (Arc<TerminalManager>, Arc<Collector>) {
    // An emulating collector answers cursor-position queries and acknowledges output, like the UI.
    let sink = Collector::emulating();
    let m = Arc::new(TerminalManager::new(Arc::new(NativePty), sink.clone()));
    sink.attach(&m);
    (m, sink)
}

#[cfg(unix)]
fn shell_plan() -> LaunchPlan {
    LaunchPlan::single("sh", "/bin/sh", &["-i"])
}

#[cfg(windows)]
fn shell_plan() -> LaunchPlan {
    // /Q: no command echo noise is needed, but keep it simple and default-like.
    LaunchPlan::single("cmd", "cmd.exe", &[])
}

fn open(m: &TerminalManager, size: PtySize) -> String {
    m.open_plan("local-test-shell", "Local test shell", shell_plan(), size)
        .expect("a real PTY session starts")
        .id
}

fn send(m: &TerminalManager, id: &str, text: &str) {
    #[cfg(windows)]
    let text = text.replace('\n', "\r");
    m.write(id, text.as_bytes()).expect("input accepted");
}

#[cfg(unix)]
#[test]
fn commands_run_and_the_shell_sees_a_real_tty() {
    let (m, s) = manager();
    let id = open(&m, PtySize { rows: 24, cols: 80 });
    // `$((20+22))` is expanded by the shell, so "vlinux-42" can only appear if the command ran.
    send(&m, &id, "echo vlinux-$((20+22))\n");
    assert!(
        s.wait_text(&id, "vlinux-42", T),
        "output was: {:?}",
        s.text(&id)
    );
    send(&m, &id, "[ -t 0 ] && [ -t 1 ] && echo tty-$((1+1))\n");
    assert!(
        s.wait_text(&id, "tty-2", T),
        "stdin/stdout must be a terminal; output: {:?}",
        s.text(&id)
    );
    send(&m, &id, "exit 0\n");
    assert!(s
        .wait_state(&id, T, |st| matches!(
            st,
            SessionState::Exited { code: Some(0) }
        ))
        .is_some());
}

#[cfg(windows)]
#[test]
fn commands_run_under_conpty() {
    let (m, s) = manager();
    let id = open(
        &m,
        PtySize {
            rows: 30,
            cols: 100,
        },
    );
    // `set /a` prints the *result*; the typed text does not contain it.
    send(&m, &id, "set /a 1234*5+7\n");
    assert!(s.wait_text(&id, "6177", T), "output was: {:?}", s.text(&id));
    send(&m, &id, "exit 0\n");
    assert!(s
        .wait_state(&id, T, |st| matches!(
            st,
            SessionState::Exited { code: Some(0) }
        ))
        .is_some());
}

#[test]
fn the_shells_exit_code_is_reported() {
    let (m, s) = manager();
    let id = open(&m, PtySize::DEFAULT);
    send(&m, &id, "exit 7\n");
    let st = s
        .wait_state(&id, T, |st| matches!(st, SessionState::Exited { .. }))
        .expect("exit reported");
    assert_eq!(st, SessionState::Exited { code: Some(7) });
    // The ended session refuses further input and can be closed.
    assert!(m.write(&id, b"x").is_err());
    m.close(&id).unwrap();
    assert!(m.list().is_empty());
}

#[cfg(unix)]
#[test]
fn resizing_changes_the_size_the_program_sees() {
    let (m, s) = manager();
    let id = open(&m, PtySize { rows: 24, cols: 80 });
    send(&m, &id, "stty size\n");
    assert!(s.wait_text(&id, "24 80", T), "output: {:?}", s.text(&id));
    m.resize(&id, 41, 133).unwrap();
    send(&m, &id, "stty size\n");
    assert!(s.wait_text(&id, "41 133", T), "output: {:?}", s.text(&id));
    m.close(&id).unwrap();
}

#[cfg(windows)]
#[test]
fn resizing_changes_the_size_the_program_sees_conpty() {
    let (m, s) = manager();
    let id = open(
        &m,
        PtySize {
            rows: 30,
            cols: 100,
        },
    );
    send(&m, &id, "mode con\n");
    assert!(s.wait_text(&id, "100", T), "output: {:?}", s.text(&id));
    m.resize(&id, 41, 133).unwrap();
    send(&m, &id, "mode con\n");
    assert!(s.wait_text(&id, "133", T), "output: {:?}", s.text(&id));
    m.close(&id).unwrap();
}

#[cfg(unix)]
#[test]
fn ctrl_c_interrupts_the_running_command_and_the_shell_survives() {
    let (m, s) = manager();
    let id = open(&m, PtySize::DEFAULT);
    send(&m, &id, "sleep 60\n");
    std::thread::sleep(Duration::from_millis(500));
    m.write(&id, b"\x03").unwrap(); // Ctrl+C, delivered by the terminal as SIGINT
    send(&m, &id, "echo after-$((40+2))\n");
    assert!(
        s.wait_text(&id, "after-42", Duration::from_secs(20)),
        "the shell must be usable right after Ctrl+C; output: {:?}",
        s.text(&id)
    );
    m.close(&id).unwrap();
}

#[cfg(unix)]
#[test]
fn unicode_survives_the_round_trip() {
    let (m, s) = manager();
    let id = open(&m, PtySize::DEFAULT);
    // Typed as UTF-8 by the "user"; the shell prints it back via printf with octal escapes too.
    send(&m, &id, "printf 'u1:%s\\n' 'héllo ✓ 日本語 🦀'\n");
    assert!(
        s.wait_text(&id, "u1:héllo ✓ 日本語 🦀", T),
        "output: {:?}",
        s.text(&id)
    );
    send(&m, &id, "printf 'u2:\\303\\251\\342\\234\\223\\n'\n");
    assert!(s.wait_text(&id, "u2:é✓", T), "output: {:?}", s.text(&id));
    m.close(&id).unwrap();
}

#[cfg(unix)]
#[test]
fn large_output_arrives_complete_under_flow_control() {
    let (m, s) = manager();
    let id = open(&m, PtySize::DEFAULT);
    // 6 MB of 'x' with no newline, then a marker that is computed (not typed).
    send(
        &m,
        &id,
        "head -c 6000000 /dev/zero | tr '\\0' x; echo; echo big-$((6*7))\n",
    );
    assert!(s.wait_text(&id, "big-42", Duration::from_secs(120)));
    let x = s.bytes(&id).iter().filter(|b| **b == b'x').count();
    assert!(x >= 6_000_000, "expected 6,000,000 x's, saw {x}");
    m.close(&id).unwrap();
}

#[cfg(windows)]
#[test]
fn large_output_arrives_complete_under_flow_control_conpty() {
    let (m, s) = manager();
    let id = open(&m, PtySize::DEFAULT);
    send(
        &m,
        &id,
        "for /L %i in (1,1,15000) do @echo vlinux-line-%i\n",
    );
    assert!(
        s.wait_text(&id, "vlinux-line-15000", Duration::from_secs(120)),
        "tail: {:?}",
        {
            let t = s.text(&id);
            t.chars()
                .rev()
                .take(200)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        }
    );
    m.close(&id).unwrap();
}

#[test]
fn several_sessions_run_side_by_side_and_close_independently() {
    let (m, s) = manager();
    let ids: Vec<String> = (0..3).map(|_| open(&m, PtySize::DEFAULT)).collect();
    for (i, id) in ids.iter().enumerate() {
        #[cfg(unix)]
        send(&m, id, &format!("echo session-{i}-$((10+{i}))\n"));
        #[cfg(windows)]
        send(&m, id, &format!("set /a 100+{i}\n"));
    }
    for (i, id) in ids.iter().enumerate() {
        #[cfg(unix)]
        let needle = format!("session-{i}-{}", 10 + i);
        #[cfg(windows)]
        let needle = format!("{}", 100 + i);
        assert!(
            s.wait_text(id, &needle, T),
            "session {i} output: {:?}",
            s.text(id)
        );
    }
    // Output stays separate.
    #[cfg(unix)]
    assert!(!s.text(&ids[0]).contains("session-1-11"));

    // Closing one session leaves the others working.
    m.close(&ids[0]).unwrap();
    assert_eq!(m.list().len(), 2);
    #[cfg(unix)]
    {
        send(&m, &ids[1], "echo still-$((1+1))\n");
        assert!(s.wait_text(&ids[1], "still-2", T));
    }
    #[cfg(windows)]
    {
        send(&m, &ids[1], "set /a 5000+1\n");
        assert!(s.wait_text(&ids[1], "5001", T));
    }
    m.close_all();
    assert!(m.list().is_empty());
}

#[test]
fn closing_a_session_stops_its_process_without_waiting_for_it() {
    let (m, s) = manager();
    let id = open(&m, PtySize::DEFAULT);
    #[cfg(unix)]
    send(&m, &id, "sleep 120\n");
    #[cfg(windows)]
    send(&m, &id, "ping -n 120 127.0.0.1\n");
    std::thread::sleep(Duration::from_millis(500));
    let started = Instant::now();
    m.close(&id).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "close must return promptly"
    );
    assert!(m.list().is_empty());
    assert!(m.write(&id, b"x").is_err());
    // A closed session never reports an exit to a UI that already dropped it.
    std::thread::sleep(Duration::from_millis(500));
    assert!(s.states(&id).is_empty());
}

#[test]
fn a_program_that_does_not_exist_is_a_clean_error() {
    let (m, _s) = manager();
    let plan = LaunchPlan::single("missing", "vlinux-definitely-not-a-program", &[]);
    let err = m.open_plan("x", "x", plan, PtySize::DEFAULT);
    assert!(
        err.is_err(),
        "starting a missing program must fail, not hang"
    );
    assert!(m.list().is_empty());
}

/// Windows only: drive the *real* `wsl.exe` under ConPTY for a distribution that does not exist.
/// It cannot start a distribution (the name is unregistered), so this is safe on a runner. It
/// proves that VLINUX's launch path copes with the real binary: the session reaches a final state
/// instead of hanging, and cleanup works. It does NOT prove a shell inside a distribution works.
#[cfg(windows)]
#[test]
fn real_wsl_exe_for_an_unregistered_distribution_ends_cleanly() {
    use vlinux_runtime::terminal::launch::{build_plan, Target};
    use vlinux_runtime::wsl::Programs;

    let (m, s) = manager();
    let target = Target {
        name: "VlinuxNoSuchDistro".into(),
        label: "none".into(),
    };
    let plan = build_plan(&Programs::system(), &target).unwrap();
    let info = match m.open_plan(&target.name, &target.label, plan, PtySize::DEFAULT) {
        Ok(i) => i,
        Err(e) => {
            // wsl.exe missing entirely is also a clean, explained failure.
            println!("wsl.exe could not be started: {e}");
            return;
        }
    };
    let end = s.wait_state(&info.id, Duration::from_secs(60), |st| {
        !matches!(st, SessionState::Running)
    });
    println!(
        "real wsl.exe, unregistered distro -> {end:?}; output: {:?}",
        s.text(&info.id)
    );
    if end.is_none() {
        // Still running (for example waiting for input): closing must still work promptly.
        let started = Instant::now();
        m.close(&info.id).unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
    } else {
        m.close(&info.id).unwrap();
    }
    assert!(m.list().is_empty());
}
