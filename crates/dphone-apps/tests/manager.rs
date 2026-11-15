//! The runtime's own rules: capability gating, time and memory limits,
//! storage locations, timers, packages, and installation.

mod common;

use common::{Rig, staged, temp_dir};
use dphone_apps::testutil::FakeConfig;
use dphone_apps::{InstallError, package};
use dphone_state::{Command, Event, Reply};
use std::path::{Path, PathBuf};

fn rig() -> Rig {
    Rig::new(FakeConfig::dasp())
}

/// Copies a staged app into `root` under a new name, editing its manifest.
fn variant(root: &Path, from: &str, name: &str, edit: impl Fn(&mut serde_json::Value)) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(staged().join(from)).unwrap().flatten() {
        std::fs::copy(e.path(), dir.join(e.file_name())).unwrap();
    }
    let mut m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("app.json")).unwrap()).unwrap();
    m["name"] = name.into();
    edit(&mut m);
    std::fs::write(dir.join("app.json"), m.to_string()).unwrap();
    dir
}

fn pack(dir: &Path, out: &Path) {
    package::pack(dir, std::fs::File::create(out).unwrap()).unwrap();
}

#[test]
fn publishes_the_app_list() {
    let r = rig();
    let apps = r.store.get::<dphone_state::Apps>();
    let names: Vec<&str> = apps.installed.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "boxpusher",
            "calendar",
            "contacts",
            "diagnostics",
            "faulty",
            "hotdesking",
            "parking",
            "probe",
            "queues",
            "status",
            "voicemail"
        ]
    );
    let vm = apps
        .installed
        .iter()
        .find(|a| a.name == "voicemail")
        .unwrap();
    assert_eq!(
        (vm.display_name.as_str(), vm.builtin, vm.running),
        ("Voicemail", true, false)
    );
}

#[test]
fn disabled_builtins_are_not_listed_or_launchable() {
    let cfg = FakeConfig::dasp();
    let disabled = cfg.disabled.clone();
    disabled.lock().unwrap().push("parking".into());
    let mut r = Rig::new(cfg);
    let listed = |r: &Rig| {
        let apps = r.store.get::<dphone_state::Apps>();
        let names = |l: &[dphone_state::AppInfo]| -> Vec<String> {
            l.iter().map(|a| a.name.clone()).collect()
        };
        (names(&apps.installed), names(&apps.disabled))
    };
    let (on, off) = listed(&r);
    assert!(!on.contains(&"parking".to_string()));
    assert_eq!(off, ["parking"]);
    assert!(r.mgr.launch("parking").is_err());
    // An app key reaches the manager as this command.
    let pending = r.store.send(Command::LaunchApp {
        name: "parking".into(),
    });
    r.settle();
    assert!(pending.wait().is_err());
    assert_eq!(r.ui.top(), None);

    // Switched off while it is on screen: it stops and leaves the list.
    r.launch("calendar");
    assert_eq!(r.ui.top().as_deref(), Some("calendar"));
    *disabled.lock().unwrap() = vec!["calendar".into()];
    r.store.emit(Event::ConfigApplied { generation: 2 });
    r.settle();
    assert!(!r.mgr.is_running("calendar"));
    assert_eq!(r.ui.top(), None);
    let (on, off) = listed(&r);
    assert!(on.contains(&"parking".to_string()) && !on.contains(&"calendar".to_string()));
    assert_eq!(off, ["calendar"]);
    r.launch("parking");
    assert_eq!(r.ui.top().as_deref(), Some("parking"));
}

#[test]
fn launch_command_and_foreground_state() {
    let mut r = rig();
    let pending = r.store.send(Command::LaunchApp {
        name: "hotdesking".into(),
    });
    r.settle();
    assert_eq!(pending.wait(), Ok(Reply::Done));
    assert_eq!(
        r.store.get::<dphone_state::Apps>().foreground.as_deref(),
        Some("hotdesking")
    );
    let pending = r.store.send(Command::LaunchApp {
        name: "nope".into(),
    });
    r.settle();
    assert!(pending.wait().is_err());
    let pending = r.store.send(Command::CloseApp {
        name: "hotdesking".into(),
    });
    r.settle();
    assert_eq!(pending.wait(), Ok(Reply::Done));
    assert_eq!(r.store.get::<dphone_state::Apps>().foreground, None);
    assert!(!r.ui.has_window("hotdesking"));
}

#[test]
fn an_app_gets_only_declared_capabilities() {
    let dir = temp_dir("gating");
    // Status uses the PBX; without the capability it must not load.
    variant(&dir.0, "status", "nopbx", |m| {
        m["capabilities"] = serde_json::json!(["ui", "config", "presence"])
    });
    variant(&dir.0, "status", "full", |_| {});
    let mut r = Rig::with_root(&dir.0, FakeConfig::dasp(), dphone_state::Model::D60);
    let err = r.mgr.launch("nopbx").unwrap_err();
    assert!(err.contains("pbx"), "{err}");
    assert!(!r.mgr.is_running("nopbx"));
    assert!(
        !r.ui.has_window("nopbx"),
        "the window of a failed start is closed"
    );
    r.launch("full");
    assert!(r.mgr.is_running("full"));
}

#[test]
fn a_spinning_app_is_stopped() {
    let mut r = rig();
    r.launch("faulty");
    assert_eq!(r.ui.texts("faulty"), ["ready"]);
    let t = std::time::Instant::now();
    r.softkey("faulty", "Spin");
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
    assert!(!r.mgr.is_running("faulty"));
    assert!(!r.ui.has_window("faulty"));
    assert_eq!(r.store.get::<dphone_state::Apps>().foreground, None);
    // It can be started again afterwards.
    r.launch("faulty");
    assert_eq!(r.ui.texts("faulty"), ["ready"]);
}

#[test]
fn a_memory_hog_is_stopped() {
    let mut r = rig();
    r.launch("faulty");
    r.softkey("faulty", "Hog");
    assert!(!r.mgr.is_running("faulty"));
}

#[test]
fn storage_follows_the_stock_rules() {
    let mut r = rig();
    r.launch("faulty");
    r.softkey("faulty", "Store");
    assert_eq!(
        r.ui.texts("faulty"),
        [
            "write=true read=true list=a.txt escape=false slash=false rofs=false pkg=true shared=true remove=true gone=true"
        ]
    );
    assert!(r.tmp.join("appdata/@shared/s.txt").is_file());
    assert!(r.nvdata.join("appdata/faulty").is_dir());
    assert!(!r.nvdata.join("appdata/b.txt").exists());
}

/// Answers the audio and UI command domains the way the phone's
/// subsystems do, recording what arrived.
fn audio_and_ui(r: &Rig) -> std::sync::Arc<std::sync::Mutex<Vec<Command>>> {
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    common::answer_commands(&r.store, dphone_state::Domain::Audio, log.clone(), |c| match c {
        Command::PlayFile { path, .. } if !Path::new(path).is_file() => {
            Err(dphone_state::CommandError::Failed("no such file".into()))
        }
        _ => Ok(Reply::Done),
    });
    common::answer_commands(&r.store, dphone_state::Domain::Ui, log.clone(), |_| Ok(Reply::Done));
    log
}

#[test]
fn dial_passes_its_headers() {
    let mut r = rig();
    r.launch("probe");
    r.softkey("probe", "Dial");
    assert_eq!(r.ui.texts("probe"), ["dial=Ok(8)"]);
    assert!(r.commands().contains(&Command::DialWith {
        target: "8500".into(),
        account: Some(dphone_state::AccountId(1)),
        headers: vec![("X-Digium-Call-Feature".into(), "voicemail".into()), ("X-Probe".into(), "1".into())],
    }));
}

#[test]
fn a_sound_file_plays_and_reports_where_it_stopped() {
    let mut r = rig();
    let log = audio_and_ui(&r);
    r.launch("probe");
    r.softkey("probe", "Play");
    assert_eq!(r.ui.texts("probe"), ["play=Ok(())"]);
    let played = log.lock().unwrap().clone();
    let [Command::PlayFile { path, start_secs: 3, app }] = &played[..] else { panic!("{played:?}") };
    assert_eq!(app, "probe");
    assert_eq!(Path::new(path), staged().join("probe/tone.wav"));
    assert!(std::fs::read(path).unwrap().starts_with(b"RIFF"));

    // The audio side says where it ended: the app hears about its own
    // file, and not about another app's.
    r.store.emit(dphone_state::Event::PlaybackStopped { app: "voicemail".into(), position_secs: 9 });
    r.settle();
    assert_eq!(r.ui.texts("probe"), ["play=Ok(())"]);
    r.store.emit(dphone_state::Event::PlaybackStopped { app: "probe".into(), position_secs: 4 });
    r.settle();
    assert_eq!(r.ui.texts("probe"), ["stopped=4"]);

    // A file that is not there is refused here; nothing is asked of the
    // audio side.
    r.softkey("probe", "Bad");
    assert_eq!(r.ui.texts("probe"), ["play=Err(Error::NotFound)"]);
    assert_eq!(log.lock().unwrap().len(), 1);

    // Stop goes to the audio side; closing an app whose file is playing
    // stops the file.
    r.softkey("probe", "Stop");
    r.softkey("probe", "Play");
    std::thread::sleep(std::time::Duration::from_millis(20));
    r.mgr.close("probe");
    std::thread::sleep(std::time::Duration::from_millis(20));
    let tail: Vec<Command> = log.lock().unwrap()[1..].to_vec();
    assert!(matches!(&tail[..], [Command::StopSound, Command::PlayFile { .. }, Command::StopSound]), "{tail:?}");
}

#[test]
fn idle_screen_events_and_the_idle_timer() {
    let mut r = rig();
    let log = audio_and_ui(&r);
    r.mgr.autostart();
    r.settle();
    assert!(r.mgr.is_running("probe"), "an app with an idle window starts by itself");
    assert_eq!(r.ui.idle_texts("probe"), ["idle=unknown"], "nothing is announced before the idle screen shows");
    assert_eq!(r.store.get::<dphone_state::Apps>().idle_app, None);
    r.ui.idle_screen("probe", true);
    r.settle();
    assert_eq!(r.ui.idle_texts("probe"), ["idle=shown"]);
    assert_eq!(r.store.get::<dphone_state::Apps>().idle_app.as_deref(), Some("probe"));
    r.ui.idle_screen("probe", false);
    r.settle();
    assert_eq!(r.ui.idle_texts("probe"), ["idle=hidden"]);
    assert_eq!(r.store.get::<dphone_state::Apps>().idle_app, None);
    r.ui.idle_screen("probe", true);
    r.settle();
    assert_eq!(r.ui.idle_texts("probe"), ["idle=shown"]);

    r.launch("probe");
    r.softkey("probe", "Awake");
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(log.lock().unwrap().clone(), [Command::ResetIdleTimer]);
}

#[test]
fn package_pictures_reach_the_window_and_the_launcher() {
    let mut r = rig();
    // The manifest's icon is a file of the package: the launcher gets its path.
    let apps = r.store.get::<dphone_state::Apps>();
    let probe = apps.installed.iter().find(|a| a.name == "probe").unwrap();
    assert_eq!(probe.icon, "pic.png");
    assert_eq!(Path::new(&probe.icon_path), staged().join("probe/pic.png"));
    // A built-in's icon is one of the phone's own: no file.
    let contacts = apps.installed.iter().find(|a| a.name == "contacts").unwrap();
    assert_eq!(contacts.icon_path, "");

    r.launch("probe");
    r.softkey("probe", "Pics");
    let size = std::fs::metadata(staged().join("probe/pic.png")).unwrap().len() as usize;
    // Read once, although an image widget and a list row both name it; the
    // name that is no file stays an icon of the phone.
    assert_eq!(r.ui.images("probe"), [("pic.png".to_string(), size)]);
    assert_eq!(r.ui.image_names("probe"), ["pic.png"]);
}

#[test]
fn timers_fire() {
    let mut r = rig();
    r.launch("faulty");
    r.softkey("faulty", "Timer");
    let wait = r.mgr.pump().expect("a timer is pending");
    assert!(wait <= std::time::Duration::from_millis(20));
    std::thread::sleep(std::time::Duration::from_millis(30));
    r.settle();
    assert_eq!(r.ui.texts("faulty"), ["ticks=1"]);
    assert_eq!(r.mgr.pump(), None, "a one-shot timer is gone after firing");
}

#[test]
fn install_run_upgrade_and_remove() {
    let dir = temp_dir("install");
    let src = variant(&dir.0, "hotdesking", "deskapp", |m| {
        m["capabilities"] = serde_json::json!(["ui"]);
        m["displayName"] = "Desk".into();
    });
    // A package carries the component, never compiled code.
    std::fs::remove_file(src.join("hotdesking.cwasm")).unwrap();
    let pkg = dir.0.join("deskapp.zip");
    pack(&src, &pkg);

    let mut r = rig();
    // Needs `hotdesk`, which the manifest no longer declares.
    let err = r.mgr.install(&pkg, None).unwrap_err();
    assert!(matches!(err, InstallError::Unloadable(_)), "{err}");
    assert!(!r.nvdata.join("wasmapps/deskapp").exists());

    // The same code declaring a built-in-only capability is refused too.
    let src2 = variant(&dir.0, "hotdesking", "deskapp2", |_| {});
    std::fs::remove_file(src2.join("hotdesking.cwasm")).unwrap();
    let pkg2 = dir.0.join("deskapp2.zip");
    pack(&src2, &pkg2);
    let err = r.mgr.install(&pkg2, None).unwrap_err();
    assert!(err.to_string().contains("reserved for built-in"), "{err}");

    // A well-formed package installs through the command bus and runs.
    let src3 = variant(&dir.0, "status", "mystatus", |m| {
        m["displayName"] = "My Status".into()
    });
    std::fs::remove_file(src3.join("status.cwasm")).unwrap();
    let pkg3 = dir.0.join("mystatus.zip");
    pack(&src3, &pkg3);
    assert!(matches!(
        r.mgr.install(&pkg3, Some("other")).unwrap_err(),
        InstallError::NameMismatch { .. }
    ));
    let pending = r.store.send(Command::InstallApp {
        path: pkg3.to_string_lossy().into_owned(),
    });
    r.settle();
    assert_eq!(pending.wait(), Ok(Reply::Text("mystatus".into())));
    let apps = r.store.get::<dphone_state::Apps>();
    let info = apps
        .installed
        .iter()
        .find(|a| a.name == "mystatus")
        .unwrap();
    assert_eq!(
        (info.display_name.as_str(), info.builtin),
        ("My Status", false)
    );
    assert!(r.nvdata.join("wasmapps/mystatus/status.wasm").is_file());
    assert!(
        !r.nvdata.join("jsapps").exists(),
        "the stock app directory is left alone"
    );
    r.launch("mystatus");
    assert!(r.mgr.is_running("mystatus"));

    // Upgrading replaces it in place (and stops the running copy first).
    r.mgr.install(&pkg3, Some("mystatus")).unwrap();
    assert!(r.nvdata.join("wasmapps/mystatus/app.json").is_file());
    let leftovers: Vec<_> = std::fs::read_dir(r.nvdata.join("wasmapps"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["mystatus"], "no staging directories remain");

    // Its data goes with it on removal.
    std::fs::create_dir_all(r.nvdata.join("appdata/mystatus")).unwrap();
    r.mgr.remove("mystatus").unwrap();
    assert!(!r.nvdata.join("wasmapps/mystatus").exists());
    assert!(!r.nvdata.join("appdata/mystatus").exists());
    assert!(
        r.mgr.remove("status").is_err(),
        "built-ins cannot be removed"
    );
}

#[test]
fn install_refuses_shadowing_javascript_and_bad_archives() {
    let dir = temp_dir("refuse");
    let mut r = rig();

    let src = variant(&dir.0, "status", "status", |_| {});
    std::fs::remove_file(src.join("status.cwasm")).unwrap();
    let pkg = dir.0.join("status.zip");
    pack(&src, &pkg);
    let err = r.mgr.install(&pkg, None).unwrap_err();
    assert!(matches!(err, InstallError::ShadowsBuiltin(_)));
    assert_eq!(err.stock_status(), -103);

    // A stock JavaScript package.
    let js = dir.0.join("js.zip");
    {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::fs::File::create(&js).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("demo/app.json", o).unwrap();
        z.write_all(br#"{"name":"demo","jsFiles":["demo.js"],"type":"foreground"}"#)
            .unwrap();
        z.start_file("demo/demo.js", o).unwrap();
        z.write_all(b"digium.background();").unwrap();
        z.finish().unwrap();
    }
    let err = r.mgr.install(&js, None).unwrap_err();
    assert!(err.to_string().contains("JavaScript"), "{err}");

    // Path traversal and compiled code are refused before anything is read.
    for (name, why) in [
        ("../evil/app.json", "escape"),
        ("x/app.cwasm", "compiled code"),
    ] {
        let bad = dir.0.join("bad.zip");
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("x/app.json", o).unwrap();
        z.write_all(br#"{"name":"x","wasm":"x.wasm"}"#).unwrap();
        z.start_file("x/x.wasm", o).unwrap();
        z.write_all(b"\0asm").unwrap();
        z.start_file(name, o).unwrap();
        z.write_all(b"data").unwrap();
        z.finish().unwrap();
        assert!(r.mgr.install(&bad, None).is_err(), "{why}");
    }
    assert!(
        std::fs::read_dir(r.nvdata.join("wasmapps"))
            .map(|d| d.count())
            .unwrap_or(0)
            == 0
    );
}

#[test]
fn stock_javascript_apps_on_nvdata_are_skipped() {
    let mut r = rig();
    let js = r.nvdata.join("jsapps/oldapp");
    std::fs::create_dir_all(&js).unwrap();
    std::fs::write(js.join("app.json"), r#"{"name":"oldapp","jsFiles":"a.js"}"#).unwrap();
    r.mgr.scan();
    assert!(
        !r.store
            .get::<dphone_state::Apps>()
            .installed
            .iter()
            .any(|a| a.name == "oldapp")
    );
    assert!(
        js.join("app.json").is_file(),
        "left in place for a flash back to stock"
    );
}
