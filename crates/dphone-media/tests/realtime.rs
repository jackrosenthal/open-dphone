// SPDX-License-Identifier: GPL-2.0-or-later
//! A refused `SCHED_FIFO` request is reported, not swallowed.

use std::sync::mpsc;
use std::time::Duration;

use dphone_media::fake::FakeAudio;
use dphone_media::{Engine, EngineOptions, Event, VpmDsp, VpmDspOptions};
use dphone_vpm::{Family, MockTransport, Vpm};

#[test]
fn a_refused_realtime_request_is_an_event() {
    let family = Family::New;
    let (audio, _state) = FakeAudio::new(family.device_rate());
    let dsp = VpmDsp::new(Vpm::new(MockTransport::new(family)).unwrap(), VpmDspOptions::default());
    let (tx, rx) = mpsc::channel();
    // No kernel grants priority 1000 (the range ends at 99), whatever the
    // privileges of the test process: the request fails with EINVAL.
    let engine = Engine::spawn(Box::new(audio), Box::new(dsp), move |e| drop(tx.send(e)), EngineOptions { rt_priority: Some(1000), queue: 8 })
        .unwrap();
    let first = rx.recv_timeout(Duration::from_secs(5)).expect("no event from the audio thread");
    assert_eq!(first, Event::RealtimeRefused { priority: 1000, errno: libc::EINVAL });
    engine.shutdown();
}
