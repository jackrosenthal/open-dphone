//! The runtime's own rules: capability gating, time and memory limits,
//! storage, timers, packages, and installation.

mod common;

use common::{Rig, staged, temp_dir};
use dphone_apps::testutil::FakeConfig;
use dphone_apps::manifest::{self, Manifest};
use dphone_apps::{InstallError, ManifestError};
use dphone_state::{Command, Event, Reply};
use std::path::{Path, PathBuf};

fn rig() -> Rig {
    Rig::new(FakeConfig::dasp())
}

/// Makes an app file of a staged app under a new name, with its manifest
/// edited: the component, with another manifest section.
fn variant(root: &Path, from: &str, name: &str, edit: impl Fn(&mut Manifest)) -> PathBuf {
    let component = std::fs::read(staged().join(from).join(format!("{from}.wasm"))).unwrap();
    let mut m = Manifest::from_wasm(&component).unwrap();
    m.name = name.into();
    edit(&mut m);
    let out = root.join(format!("{name}.wasm"));
    std::fs::write(
        &out,
        manifest::with_section(&component, &m.section()).unwrap(),
    )
    .unwrap();
    out
}

/// A data file the test guest `probe` has in its code.
fn probe_data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/guests/probe/data").join(name)
}

/// A module that holds nothing but this manifest section.
fn bare_module(section: &[u8]) -> Vec<u8> {
    let name = manifest::SECTION.as_bytes();
    let mut m = b"\0asm\x01\0\0\0\0".to_vec();
    assert!(name.len() + 1 + section.len() < 128);
    m.push((name.len() + 1 + section.len()) as u8);
    m.push(name.len() as u8);
    m.extend_from_slice(name);
    m.extend_from_slice(section);
    m
}

#[test]
fn publishes_the_app_list() {
    let r = rig();
    let apps = r.store.get::<dphone_state::Apps>();
    let names: Vec<&str> = apps.installed.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "alarms",
            "boxpusher",
            "calendar",
            "checks",
            "contacts",
            "desksparrow",
            "diagnostics",
            "faulty",
            "hotdesking",
            "lightup",
            "parking",
            "probe",
            "queues",
            "status",
            "storeprobe",
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

/// A staged app again in `root` under a new name, as a built-in app of a
/// phone whose apps are there, with the phone's record of it edited.
fn built_in_variant(root: &Path, from: &str, name: &str, edit: impl Fn(&mut Manifest)) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        staged().join(from).join(format!("{from}.cwasm")),
        dir.join(format!("{name}.cwasm")),
    )
    .unwrap();
    let mut record = common::record(from);
    record.name = name.into();
    edit(&mut record);
    std::fs::write(dir.join(manifest::RECORD), record.record()).unwrap();
}

#[test]
fn an_app_gets_only_the_interfaces_in_its_record() {
    let dir = temp_dir("gating");
    // The phone's record of an app lists the interfaces its component
    // imports, and the app is linked with those alone.  Status uses the
    // PBX: with a record that lacks `pbx` it does not load.
    built_in_variant(&dir.0, "status", "nopbx", |m| {
        m.capabilities.retain(|c| *c != dphone_apps::Capability::Pbx)
    });
    built_in_variant(&dir.0, "status", "full", |_| {});
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
fn an_app_has_a_store_of_keys_and_values() {
    let mut r = rig();
    r.launch("faulty");
    r.softkey("faulty", "Store");
    assert_eq!(
        r.ui.texts("faulty"),
        [
            "set=true get=true keys=a.txt,b escape=false slash=false dot=false remove=true gone=true again=true"
        ]
    );
    // A key is a file in the app's directory, and nowhere else.
    assert_eq!(std::fs::read(r.nvdata.join("appdata/faulty/b")).unwrap(), b"2");
    assert!(!r.nvdata.join("appdata/faulty/a.txt").exists());
    assert!(r.nvdata.join("appdata/faulty").is_dir());
    assert!(!r.nvdata.join("appdata/b.txt").exists());
}

/// Answers the audio and UI command domains the way the phone's
/// subsystems do, recording what arrived.
fn audio_and_ui(r: &Rig) -> std::sync::Arc<std::sync::Mutex<Vec<Command>>> {
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    common::answer_commands(&r.store, dphone_state::Domain::Audio, log.clone(), |_| Ok(Reply::Done));
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
fn a_sound_plays_and_reports_where_it_stopped() {
    let mut r = rig();
    let log = audio_and_ui(&r);
    r.launch("probe");
    r.softkey("probe", "Play");
    assert_eq!(r.ui.texts("probe"), ["play=Ok(())"]);
    let played = log.lock().unwrap().clone();
    // The sound is the app's own data, handed over as it is.
    let [Command::PlayData { sound, start_secs: 3, app }] = &played[..] else { panic!("{played:?}") };
    assert_eq!(app, "probe");
    assert_eq!(sound.0, std::fs::read(probe_data("tone.wav")).unwrap());
    assert!(sound.0.starts_with(b"RIFF"));

    // The audio side says where it ended: the app hears about its own
    // file, and not about another app's.
    r.store.emit(dphone_state::Event::PlaybackStopped { app: "voicemail".into(), position_secs: 9 });
    r.settle();
    assert_eq!(r.ui.texts("probe"), ["play=Ok(())"]);
    r.store.emit(dphone_state::Event::PlaybackStopped { app: "probe".into(), position_secs: 4 });
    r.settle();
    assert_eq!(r.ui.texts("probe"), ["stopped=4"]);

    // The app's file is over: Stop asks nothing of the audio side, where
    // another sound may be playing.  Stop while the file plays goes
    // there, and closing an app whose file is playing stops the file.
    r.softkey("probe", "Stop");
    r.softkey("probe", "Play");
    r.softkey("probe", "Stop");
    r.softkey("probe", "Play");
    std::thread::sleep(std::time::Duration::from_millis(20));
    r.mgr.close("probe");
    std::thread::sleep(std::time::Duration::from_millis(20));
    let tail: Vec<Command> = log.lock().unwrap()[1..].to_vec();
    assert!(
        matches!(
            &tail[..],
            [Command::PlayData { .. }, Command::StopSound, Command::PlayData { .. }, Command::StopSound]
        ),
        "{tail:?}"
    );
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
fn an_apps_pictures_reach_the_window_and_the_launcher() {
    let mut r = rig();
    let pic = std::fs::read(probe_data("pic.png")).unwrap();
    // The manifest's icon is in the phone's record of the app, and the
    // launcher gets it from there.
    let apps = r.store.get::<dphone_state::Apps>();
    let probe = apps.installed.iter().find(|a| a.name == "probe").unwrap();
    assert_eq!(probe.icon, "");
    assert_eq!(&probe.icon_png.0[..], &pic[..]);
    // A built-in's icon is one of the phone's own, by name.
    let contacts = apps.installed.iter().find(|a| a.name == "contacts").unwrap();
    assert_eq!(contacts.icon, "contacts");
    assert!(contacts.icon_png.is_empty());
    // Nothing of an app's is unpacked: its directory holds the phone's
    // record of it and its code.
    let mut files: Vec<String> = std::fs::read_dir(staged().join("probe"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["app.pb", "probe.cwasm", "probe.wasm"]);

    r.launch("probe");
    r.softkey("probe", "Pics");
    assert_eq!(r.ui.texts("probe"), ["pics=Ok(())"]);
    // The app defined the picture once, from its own data; an image widget
    // and a list row both name it, and the name the app did not define
    // stays an icon of the phone.
    assert_eq!(r.ui.images("probe"), [("pic.png".to_string(), pic.len())]);
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
    let mut r = rig();

    // A well-formed app file installs through the command bus and runs.
    let pkg3 = variant(&dir.0, "status", "mystatus", |m| {
        m.display_names.text = "My Status".into()
    });
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
    // In its directory: the component, the phone's record of it (with the
    // interfaces the component imports), and the code the phone compiled.
    let installed = r.nvdata.join("wasmapps/mystatus");
    assert!(installed.join("mystatus.wasm").is_file());
    assert!(installed.join(".code.cwasm").is_file());
    let record = Manifest::from_record(&std::fs::read(installed.join("app.pb")).unwrap()).unwrap();
    let uses: Vec<&str> = record.capabilities.iter().map(|c| c.name()).collect();
    assert_eq!(uses, ["ui", "config", "pbx", "presence"]);
    assert!(
        !r.nvdata.join("jsapps").exists(),
        "the stock app directory is left alone"
    );
    r.launch("mystatus");
    assert!(r.mgr.is_running("mystatus"));

    // Upgrading replaces it in place (and stops the running copy first).
    r.mgr.install(&pkg3, Some("mystatus")).unwrap();
    assert!(r.nvdata.join("wasmapps/mystatus/app.pb").is_file());
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
fn install_refuses_shadowing_javascript_and_what_is_not_an_app() {
    let dir = temp_dir("refuse");
    let mut r = rig();

    let pkg = variant(&dir.0, "status", "status", |_| {});
    let err = r.mgr.install(&pkg, None).unwrap_err();
    assert!(matches!(err, InstallError::ShadowsBuiltin(_)));
    assert_eq!(err.stock_status(), -103);

    let file = dir.0.join("file");
    let mut install = |bytes: &[u8]| {
        std::fs::write(&file, bytes).unwrap();
        r.mgr.install(&file, None).unwrap_err()
    };

    // A stock JavaScript package: a ZIP archive with an `app.json`.
    let err = install(b"PK\x03\x04\x14\0\0\0\0\0demo/app.json{\"name\":\"demo\",\"jsFiles\":[\"demo.js\"]}");
    assert!(err.to_string().contains("JavaScript"), "{err}");
    assert_eq!(err.stock_status(), -101);

    // Something else altogether, and WebAssembly that is not an app.
    let err = install(b"#!/bin/sh\n");
    assert!(matches!(err, InstallError::Manifest(ManifestError::NotWasm)), "{err}");
    let err = install(b"\0asm\x01\0\0\0");
    assert!(matches!(err, InstallError::Manifest(ManifestError::NoManifest)), "{err}");
    assert_eq!(err.stock_status(), -101);

    // A manifest for a newer firmware, a bad name, and an icon that is no
    // picture: refused from the manifest alone, before anything is
    // compiled or written.
    let err = install(&bare_module(&[8, 9, 0x12, 1, b'x']));
    assert!(matches!(err, InstallError::Manifest(ManifestError::Format(9))), "{err}");
    let err = install(&bare_module(&[8, 1, 0x12, 3, b'a', b'/', b'b']));
    assert_eq!(err.stock_status(), -102, "{err}");
    let mut m = Manifest::from_wasm(&bare_module(&[8, 1, 0x12, 1, b'x'])).unwrap();
    m.icon_png = b"GIF89a".to_vec();
    let err = install(&bare_module(&m.section()));
    assert!(matches!(err, InstallError::Manifest(ManifestError::BadIcon(_))), "{err}");
    assert!(
        std::fs::read_dir(r.nvdata.join("wasmapps"))
            .map(|d| d.count())
            .unwrap_or(0)
            == 0
    );
}

/// An installed app is its file and the phone's record of it: its icon is
/// in the record, its other data in its code, and nothing is unpacked.
#[test]
fn an_installed_app_carries_its_icon_and_its_data() {
    let dir = temp_dir("files");
    let mut r = rig();
    let pic = std::fs::read(probe_data("pic.png")).unwrap();
    // An icon for this model of phone (the rig is a D60) and another.
    let color = [&pic[..], b"for the color display"].concat();
    let pkg = variant(&dir.0, "probe", "myprobe", |m| {
        m.icons_for = vec![("D40".into(), pic.clone()), ("D60".into(), color.clone())];
    });
    r.mgr.install(&pkg, None).unwrap();
    let installed = r.nvdata.join("wasmapps/myprobe");
    let mut files: Vec<String> = std::fs::read_dir(&installed)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, [".code.cwasm", "app.pb", "myprobe.wasm"]);
    assert_eq!(
        std::fs::read(installed.join("myprobe.wasm")).unwrap(),
        std::fs::read(&pkg).unwrap()
    );

    let apps = r.store.get::<dphone_state::Apps>();
    let info = apps.installed.iter().find(|a| a.name == "myprobe").unwrap();
    assert_eq!(&info.icon_png.0[..], &color[..]);
    r.launch("myprobe");
    r.softkey("myprobe", "Pics");
    assert_eq!(r.ui.image_names("myprobe"), ["pic.png"]);
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

/// The command `InstallApp` compiles on a worker thread, so the manager
/// goes on with its work meanwhile; and compiled code that another
/// firmware made of an installed app is compiled again the same way.
#[test]
fn installing_compiles_off_the_manager_thread() {
    let dir = temp_dir("worker");
    let pkg = variant(&dir.0, "status", "mystatus", |_| {});

    let mut r = rig();
    let installing = r.store.send(Command::InstallApp {
        path: pkg.to_string_lossy().into_owned(),
    });
    let launching = r.store.send(Command::LaunchApp {
        name: "status".into(),
    });
    r.mgr.pump();
    // The launch was answered at once; the install is still with the
    // compiler (its result is taken by a later pump).
    assert!(r.mgr.compiling());
    assert_eq!(launching.wait(), Ok(Reply::Done));
    assert!(r.mgr.is_running("status"));
    r.settle();
    assert_eq!(installing.wait(), Ok(Reply::Text("mystatus".into())));

    // A firmware update that changes the engine leaves compiled code the
    // new firmware cannot load.
    let code = r.nvdata.join("wasmapps/mystatus/.code.cwasm");
    std::fs::write(&code, b"compiled by another firmware").unwrap();
    let err = r.mgr.launch("mystatus").unwrap_err();
    assert!(err.contains("being compiled"), "{err}");
    assert!(r.mgr.compiling());
    r.settle();
    r.launch("mystatus");
    assert!(r.mgr.is_running("mystatus"));
    assert!(std::fs::metadata(&code).unwrap().len() > 1000);
}

#[test]
fn install_refuses_a_component_too_large_to_compile() {
    let dir = temp_dir("large");
    let pkg = variant(&dir.0, "status", "large", |_| {});
    // More code than the phone compiles: here, one more module in the
    // component, with a code section of that size.
    fn leb(mut n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        while n >= 0x80 {
            out.push(n as u8 & 0x7f | 0x80);
            n >>= 7;
        }
        out.push(n as u8);
        out
    }
    let code = dphone_apps::MAX_COMPONENT as usize + 1;
    let mut module = b"\0asm\x01\0\0\0\x0a".to_vec();
    module.extend(leb(code));
    module.resize(module.len() + code, 0);
    let mut component = std::fs::read(&pkg).unwrap();
    component.push(1);
    component.extend(leb(module.len()));
    component.extend(module);
    std::fs::write(&pkg, component).unwrap();
    let mut r = rig();
    let err = r.mgr.install(&pkg, None).unwrap_err();
    assert!(
        matches!(err, InstallError::ComponentTooLarge { .. }),
        "{err}"
    );
    assert!(!r.nvdata.join("wasmapps/large").exists());
}
