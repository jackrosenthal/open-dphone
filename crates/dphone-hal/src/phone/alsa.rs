//! ALSA PCM and control devices by raw ioctl (no alsa-lib).
//!
//! Card "Digium Phone": PCM 0 speakerphone, 1 handset, 2 headset; each
//! mono S16_LE at the family rate.  Only blocking `READI`/`WRITEI` are
//! used, which keeps clear of the ioctls whose layout depends on the
//! 64-bit time ABI.

#![allow(clippy::unnecessary_cast)] // c_long is i32 on ARM, i64 on the host

use super::sys::*;
use crate::*;
use libc::{c_long, c_ulong};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const CARD_NAME: &str = "Digium Phone";

// ---- PCM structures

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SndMask {
    bits: [u32; 8],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SndInterval {
    min: u32,
    max: u32,
    /// bit 0 openmin, 1 openmax, 2 integer, 3 empty.
    flags: u32,
}

#[repr(C)]
pub(crate) struct SndPcmHwParams {
    flags: u32,
    masks: [SndMask; 3],
    mres: [SndMask; 5],
    pub(crate) intervals: [SndInterval; 12],
    ires: [SndInterval; 9],
    pub(crate) rmask: u32,
    cmask: u32,
    info: u32,
    msbits: u32,
    rate_num: u32,
    rate_den: u32,
    pub(crate) fifo_size: c_ulong,
    reserved: [u8; 64],
}

#[repr(C)]
pub(crate) struct SndPcmSwParams {
    tstamp_mode: i32,
    period_step: u32,
    sleep_min: u32,
    avail_min: c_ulong,
    xfer_align: c_ulong,
    start_threshold: c_ulong,
    stop_threshold: c_ulong,
    silence_threshold: c_ulong,
    silence_size: c_ulong,
    boundary: c_ulong,
    pub(crate) proto: u32,
    tstamp_type: u32,
    reserved: [u8; 56],
}

#[repr(C)]
pub(crate) struct SndXferi {
    result: c_long,
    buf: *mut libc::c_void,
    frames: c_ulong,
}

pub(crate) const SNDRV_PCM_IOCTL_HW_PARAMS: u32 = iowr::<SndPcmHwParams>(b'A', 0x11);
pub(crate) const SNDRV_PCM_IOCTL_SW_PARAMS: u32 = iowr::<SndPcmSwParams>(b'A', 0x13);
pub(crate) const SNDRV_PCM_IOCTL_DELAY: u32 = ior::<c_long>(b'A', 0x21);
pub(crate) const SNDRV_PCM_IOCTL_PREPARE: u32 = io(b'A', 0x40);
pub(crate) const SNDRV_PCM_IOCTL_DROP: u32 = io(b'A', 0x43);
pub(crate) const SNDRV_PCM_IOCTL_WRITEI_FRAMES: u32 = iow::<SndXferi>(b'A', 0x50);
pub(crate) const SNDRV_PCM_IOCTL_READI_FRAMES: u32 = ior::<SndXferi>(b'A', 0x51);

const HW_PARAM_ACCESS: usize = 0;
const HW_PARAM_FORMAT: usize = 1;
const HW_PARAM_SUBFORMAT: usize = 2;
const HW_PARAM_FIRST_INTERVAL: usize = 8;
const HW_PARAM_CHANNELS: usize = 10;
const HW_PARAM_RATE: usize = 11;
const HW_PARAM_PERIOD_SIZE: usize = 13;
const HW_PARAM_PERIODS: usize = 15;
const HW_PARAM_BUFFER_SIZE: usize = 17;
const ACCESS_RW_INTERLEAVED: u32 = 3;
const FORMAT_S16_LE: u32 = 2;
const SUBFORMAT_STD: u32 = 0;

impl SndPcmHwParams {
    /// "Anything goes": all masks full, all intervals 0..=MAX, as alsa-lib
    /// and tinyalsa start out.
    fn any() -> Box<SndPcmHwParams> {
        // SAFETY: the struct is plain integers; all-zero is valid.
        let mut p: Box<SndPcmHwParams> = Box::new(unsafe { std::mem::zeroed() });
        for m in p.masks.iter_mut() {
            m.bits = [u32::MAX; 8];
        }
        for i in p.intervals.iter_mut() {
            i.min = 0;
            i.max = u32::MAX;
            i.flags = 0;
        }
        p.rmask = u32::MAX;
        p.info = u32::MAX;
        p
    }
    fn set_mask(&mut self, param: usize, bit: u32) {
        let m = &mut self.masks[param];
        m.bits = [0; 8];
        m.bits[(bit / 32) as usize] = 1 << (bit % 32);
    }
    fn set_int(&mut self, param: usize, v: u32) {
        let i = &mut self.intervals[param - HW_PARAM_FIRST_INTERVAL];
        i.min = v;
        i.max = v;
        i.flags = 1 << 2;
    }
    fn get_int(&self, param: usize) -> u32 {
        self.intervals[param - HW_PARAM_FIRST_INTERVAL].min
    }
}

fn pcm_err(what: &str, e: std::io::Error) -> HalError {
    match e.raw_os_error() {
        Some(libc::EPIPE) | Some(libc::ESTRPIPE) => HalError::Xrun,
        _ => HalError::io(format!("PCM {what}"), e),
    }
}

struct Stream {
    file: File,
    buffer_frames: u32,
}

impl Stream {
    fn open(card: u32, device: PcmDevice, playback: bool, rate: u32, cfg: &PcmConfig) -> Result<(Stream, u32, u32)> {
        let path = PathBuf::from(format!(
            "/dev/snd/pcmC{card}D{}{}",
            device.index(),
            if playback { 'p' } else { 'c' }
        ));
        // Non-blocking open so a busy device fails instead of hanging;
        // then switch to blocking I/O.
        let file = open_rw(&path, libc::O_NONBLOCK).map_err(|e| HalError::io(path.display().to_string(), e))?;
        set_blocking(&file).map_err(|e| HalError::io("fcntl", e))?;
        let fd = file.as_raw_fd();

        let mut hw = SndPcmHwParams::any();
        hw.set_mask(HW_PARAM_ACCESS, ACCESS_RW_INTERLEAVED);
        hw.set_mask(HW_PARAM_FORMAT, FORMAT_S16_LE);
        hw.set_mask(HW_PARAM_SUBFORMAT, SUBFORMAT_STD);
        hw.set_int(HW_PARAM_CHANNELS, 1);
        hw.set_int(HW_PARAM_RATE, rate);
        hw.set_int(HW_PARAM_PERIOD_SIZE, cfg.period_frames);
        hw.set_int(HW_PARAM_PERIODS, cfg.periods.max(2));
        if let Err(first) = ioctl(fd, SNDRV_PCM_IOCTL_HW_PARAMS, &mut *hw) {
            // The driver may not allow that exact geometry: ask only for the
            // format and rate with at least our period, and take what comes.
            let mut hw2 = SndPcmHwParams::any();
            hw2.set_mask(HW_PARAM_ACCESS, ACCESS_RW_INTERLEAVED);
            hw2.set_mask(HW_PARAM_FORMAT, FORMAT_S16_LE);
            hw2.set_mask(HW_PARAM_SUBFORMAT, SUBFORMAT_STD);
            hw2.set_int(HW_PARAM_CHANNELS, 1);
            hw2.set_int(HW_PARAM_RATE, rate);
            hw2.intervals[HW_PARAM_PERIOD_SIZE - HW_PARAM_FIRST_INTERVAL].min = cfg.period_frames;
            if ioctl(fd, SNDRV_PCM_IOCTL_HW_PARAMS, &mut *hw2).is_err() {
                return Err(HalError::io(format!("PCM HW_PARAMS {}", path.display()), first));
            }
            hw = hw2;
        }
        let period = hw.get_int(HW_PARAM_PERIOD_SIZE);
        let buffer = hw.get_int(HW_PARAM_BUFFER_SIZE);
        let periods = hw.get_int(HW_PARAM_PERIODS);

        // Same computation as the kernel's.
        let mut boundary = buffer as c_ulong;
        while boundary * 2 <= c_long::MAX as c_ulong - buffer as c_ulong {
            boundary *= 2;
        }
        // SAFETY: plain integers; zero is valid.
        let mut sw: SndPcmSwParams = unsafe { std::mem::zeroed() };
        sw.period_step = 1;
        sw.avail_min = period as c_ulong;
        // Capture starts on the first read; playback once a period is queued.
        sw.start_threshold = if playback { period as c_ulong } else { 1 };
        sw.stop_threshold = buffer as c_ulong;
        sw.boundary = boundary;
        ioctl(fd, SNDRV_PCM_IOCTL_SW_PARAMS, &mut sw).map_err(|e| HalError::io("PCM SW_PARAMS", e))?;
        ioctl_none(fd, SNDRV_PCM_IOCTL_PREPARE).map_err(|e| HalError::io("PCM PREPARE", e))?;
        Ok((Stream { file, buffer_frames: buffer }, period, periods))
    }

    fn xfer(&mut self, req: u32, what: &str, ptr: *mut i16, frames: usize) -> Result<()> {
        let mut done = 0usize;
        while done < frames {
            let mut x = SndXferi {
                result: 0,
                // SAFETY of the pointer: `ptr` covers `frames` samples, and
                // `done < frames`.
                buf: unsafe { ptr.add(done) } as *mut libc::c_void,
                frames: (frames - done) as c_ulong,
            };
            ioctl(self.file.as_raw_fd(), req, &mut x).map_err(|e| pcm_err(what, e))?;
            if x.result <= 0 {
                return Err(HalError::io(format!("PCM {what}"), std::io::ErrorKind::UnexpectedEof.into()));
            }
            done += x.result as usize;
        }
        Ok(())
    }
}

struct AlsaPcm {
    device: PcmDevice,
    config: PcmConfig,
    capture: Option<Stream>,
    playback: Option<Stream>,
    open_set: Arc<Mutex<[bool; 3]>>,
}

impl Pcm for AlsaPcm {
    fn config(&self) -> PcmConfig {
        self.config
    }

    fn read(&mut self, buf: &mut [i16]) -> Result<()> {
        let s = self.capture.as_mut().ok_or_else(|| HalError::Invalid("PCM not opened for capture".into()))?;
        s.xfer(SNDRV_PCM_IOCTL_READI_FRAMES, "read", buf.as_mut_ptr(), buf.len())
    }

    fn write(&mut self, buf: &[i16]) -> Result<()> {
        let s = self.playback.as_mut().ok_or_else(|| HalError::Invalid("PCM not opened for playback".into()))?;
        // The kernel only reads through this pointer for WRITEI.
        s.xfer(SNDRV_PCM_IOCTL_WRITEI_FRAMES, "write", buf.as_ptr() as *mut i16, buf.len())
    }

    fn recover(&mut self) -> Result<()> {
        for s in [self.capture.as_ref(), self.playback.as_ref()].into_iter().flatten() {
            let fd = s.file.as_raw_fd();
            let _ = ioctl_none(fd, SNDRV_PCM_IOCTL_DROP);
            ioctl_none(fd, SNDRV_PCM_IOCTL_PREPARE).map_err(|e| HalError::io("PCM PREPARE", e))?;
        }
        Ok(())
    }

    fn playback_delay(&mut self) -> Result<u32> {
        let s = self.playback.as_ref().ok_or_else(|| HalError::Invalid("PCM not opened for playback".into()))?;
        let mut d: c_long = 0;
        ioctl(s.file.as_raw_fd(), SNDRV_PCM_IOCTL_DELAY, &mut d).map_err(|e| pcm_err("delay", e))?;
        Ok(d.clamp(0, s.buffer_frames as c_long) as u32)
    }
}

impl Drop for AlsaPcm {
    fn drop(&mut self) {
        for s in [self.capture.as_ref(), self.playback.as_ref()].into_iter().flatten() {
            let _ = ioctl_none(s.file.as_raw_fd(), SNDRV_PCM_IOCTL_DROP);
        }
        self.open_set.lock().unwrap()[self.device.index() as usize] = false;
    }
}

// ---- control structures

#[repr(C)]
pub(crate) struct SndCtlCardInfo {
    card: i32,
    pad: i32,
    id: [u8; 16],
    driver: [u8; 16],
    name: [u8; 32],
    longname: [u8; 80],
    reserved: [u8; 16],
    mixername: [u8; 80],
    components: [u8; 128],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SndCtlElemId {
    numid: u32,
    iface: i32,
    device: u32,
    subdevice: u32,
    name: [u8; 44],
    index: u32,
}

#[repr(C)]
pub(crate) struct SndCtlElemList {
    offset: u32,
    space: u32,
    used: u32,
    count: u32,
    pids: *mut SndCtlElemId,
    reserved: [u8; 50],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InfoInteger {
    min: c_long,
    max: c_long,
    step: c_long,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InfoInteger64 {
    min: i64,
    max: i64,
    step: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InfoEnumerated {
    items: u32,
    item: u32,
    name: [u8; 64],
    names_ptr: u64,
    names_length: u32,
}

#[repr(C)]
pub(crate) union InfoValue {
    integer: InfoInteger,
    integer64: InfoInteger64,
    enumerated: InfoEnumerated,
    reserved: [u8; 128],
}

#[repr(C)]
pub(crate) struct SndCtlElemInfo {
    id: SndCtlElemId,
    type_: i32,
    access: u32,
    count: u32,
    owner: i32,
    pub(crate) value: InfoValue,
    reserved: [u8; 64],
}

#[repr(C)]
pub(crate) union ElemValueUnion {
    integer: [c_long; 128],
    integer64: [i64; 64],
    enumerated: [u32; 128],
    bytes: [u8; 512],
}

#[repr(C)]
pub(crate) struct SndCtlElemValue {
    id: SndCtlElemId,
    indirect: u32,
    pub(crate) value: ElemValueUnion,
    reserved: [u8; 128],
}

pub(crate) const SNDRV_CTL_IOCTL_CARD_INFO: u32 = ior::<SndCtlCardInfo>(b'U', 0x01);
pub(crate) const SNDRV_CTL_IOCTL_ELEM_LIST: u32 = iowr::<SndCtlElemList>(b'U', 0x10);
pub(crate) const SNDRV_CTL_IOCTL_ELEM_INFO: u32 = iowr::<SndCtlElemInfo>(b'U', 0x11);
pub(crate) const SNDRV_CTL_IOCTL_ELEM_READ: u32 = iowr::<SndCtlElemValue>(b'U', 0x12);
pub(crate) const SNDRV_CTL_IOCTL_ELEM_WRITE: u32 = iowr::<SndCtlElemValue>(b'U', 0x13);

const ELEM_TYPE_BOOLEAN: i32 = 1;
const ELEM_TYPE_INTEGER: i32 = 2;
const ELEM_TYPE_ENUMERATED: i32 = 3;

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn open_control(card: u32) -> std::io::Result<File> {
    open_rw(&PathBuf::from(format!("/dev/snd/controlC{card}")), 0)
}

/// Finds the phone's card number: the one named "Digium Phone", else the
/// first card present.
fn find_card() -> Result<u32> {
    let mut first = None;
    for n in 0..8 {
        let Ok(f) = open_control(n) else { continue };
        first.get_or_insert(n);
        // SAFETY: plain bytes and integers; zero is valid.
        let mut info: SndCtlCardInfo = unsafe { std::mem::zeroed() };
        if ioctl(f.as_raw_fd(), SNDRV_CTL_IOCTL_CARD_INFO, &mut info).is_ok() && cstr(&info.name) == CARD_NAME {
            return Ok(n);
        }
    }
    first.ok_or_else(|| HalError::Unavailable("no ALSA card".into()))
}

struct Ctl {
    id: SndCtlElemId,
    info: ControlInfo,
    type_: i32,
}

struct AlsaMixer {
    file: File,
    ctls: Vec<Ctl>,
}

impl AlsaMixer {
    fn open(card: u32) -> Result<AlsaMixer> {
        let file = open_control(card).map_err(|e| HalError::io(format!("/dev/snd/controlC{card}"), e))?;
        let fd = file.as_raw_fd();
        // SAFETY (all zeroed structs below): plain data; zero is valid.
        let mut list: SndCtlElemList = unsafe { std::mem::zeroed() };
        ioctl(fd, SNDRV_CTL_IOCTL_ELEM_LIST, &mut list).map_err(|e| HalError::io("ELEM_LIST", e))?;
        let mut ids: Vec<SndCtlElemId> = vec![unsafe { std::mem::zeroed() }; list.count as usize];
        let mut list2: SndCtlElemList = unsafe { std::mem::zeroed() };
        list2.space = ids.len() as u32;
        list2.pids = ids.as_mut_ptr();
        ioctl(fd, SNDRV_CTL_IOCTL_ELEM_LIST, &mut list2).map_err(|e| HalError::io("ELEM_LIST", e))?;
        ids.truncate(list2.used as usize);

        let mut ctls = Vec::new();
        for id in ids {
            let mut ei: SndCtlElemInfo = unsafe { std::mem::zeroed() };
            ei.id = id;
            if ioctl(fd, SNDRV_CTL_IOCTL_ELEM_INFO, &mut ei).is_err() {
                continue;
            }
            let (kind, min, max, mut items) = match ei.type_ {
                ELEM_TYPE_BOOLEAN => (ControlKind::Boolean, 0, 1, Vec::new()),
                // SAFETY: the kernel filled the member matching `type_`.
                ELEM_TYPE_INTEGER => unsafe {
                    (ControlKind::Integer, ei.value.integer.min as i32, ei.value.integer.max as i32, Vec::new())
                },
                ELEM_TYPE_ENUMERATED => unsafe {
                    (ControlKind::Enumerated, 0, ei.value.enumerated.items as i32 - 1, Vec::new())
                },
                _ => (ControlKind::Other, 0, 0, Vec::new()),
            };
            if kind == ControlKind::Enumerated {
                for item in 0..=max.max(0) as u32 {
                    let mut e2: SndCtlElemInfo = unsafe { std::mem::zeroed() };
                    e2.id = id;
                    e2.value.enumerated = InfoEnumerated { items: 0, item, name: [0; 64], names_ptr: 0, names_length: 0 };
                    if ioctl(fd, SNDRV_CTL_IOCTL_ELEM_INFO, &mut e2).is_ok() {
                        // SAFETY: enumerated info was returned.
                        items.push(cstr(unsafe { &e2.value.enumerated.name }));
                    }
                }
            }
            ctls.push(Ctl {
                id: ei.id,
                info: ControlInfo { name: cstr(&ei.id.name), kind, count: ei.count, min, max, items },
                type_: ei.type_,
            });
        }
        Ok(AlsaMixer { file, ctls })
    }

    fn find(&self, name: &str) -> Result<&Ctl> {
        self.ctls
            .iter()
            .find(|c| c.info.name == name)
            .ok_or_else(|| HalError::Invalid(format!("no mixer control {name:?}")))
    }
}

impl Mixer for AlsaMixer {
    fn controls(&mut self) -> Result<Vec<ControlInfo>> {
        Ok(self.ctls.iter().map(|c| c.info.clone()).collect())
    }

    fn get(&mut self, name: &str) -> Result<Vec<i32>> {
        let c = self.find(name)?;
        // SAFETY: plain data; zero is valid.
        let mut v: Box<SndCtlElemValue> = Box::new(unsafe { std::mem::zeroed() });
        v.id = c.id;
        ioctl(self.file.as_raw_fd(), SNDRV_CTL_IOCTL_ELEM_READ, &mut *v)
            .map_err(|e| HalError::io(format!("read control {name:?}"), e))?;
        let n = (c.info.count as usize).min(64);
        // SAFETY: reading the union member that matches the control type.
        Ok(match c.type_ {
            ELEM_TYPE_ENUMERATED => unsafe { v.value.enumerated[..n].iter().map(|x| *x as i32).collect() },
            _ => unsafe { v.value.integer[..n].iter().map(|x| *x as i32).collect() },
        })
    }

    fn set(&mut self, name: &str, values: &[i32]) -> Result<()> {
        let c = self.find(name)?;
        if values.is_empty() {
            return Err(HalError::Invalid("no values".into()));
        }
        if !matches!(c.info.kind, ControlKind::Boolean | ControlKind::Integer | ControlKind::Enumerated) {
            return Err(HalError::Unsupported("control type"));
        }
        // SAFETY: plain data; zero is valid.
        let mut v: Box<SndCtlElemValue> = Box::new(unsafe { std::mem::zeroed() });
        v.id = c.id;
        for i in 0..(c.info.count as usize).min(64) {
            let x = values.get(i).copied().unwrap_or(values[0]).clamp(c.info.min, c.info.max);
            // SAFETY: writing the union member that matches the type.
            unsafe {
                if c.type_ == ELEM_TYPE_ENUMERATED {
                    v.value.enumerated[i] = x as u32;
                } else {
                    v.value.integer[i] = x as c_long;
                }
            }
        }
        ioctl(self.file.as_raw_fd(), SNDRV_CTL_IOCTL_ELEM_WRITE, &mut *v)
            .map_err(|e| HalError::io(format!("write control {name:?}"), e))?;
        Ok(())
    }
}

/// The phone's sound card.
pub struct AlsaAudio {
    card: u32,
    rate: u32,
    open_set: Arc<Mutex<[bool; 3]>>,
}

impl AlsaAudio {
    pub fn open(model: Model) -> Result<AlsaAudio> {
        Ok(AlsaAudio { card: find_card()?, rate: model.audio_rate(), open_set: Default::default() })
    }
}

impl Audio for AlsaAudio {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn open(&mut self, device: PcmDevice, config: PcmConfig) -> Result<Box<dyn Pcm>> {
        if config.period_frames == 0 || !(config.capture || config.playback) {
            return Err(HalError::Invalid("empty PCM configuration".into()));
        }
        {
            let mut set = self.open_set.lock().unwrap();
            for other in [PcmDevice::Speaker, PcmDevice::Handset, PcmDevice::Headset] {
                if set[other.index() as usize] && (other == device || other.conflicts_with(device)) {
                    return Err(HalError::Invalid(format!("{device:?} is busy ({other:?} is open)")));
                }
            }
            set[device.index() as usize] = true;
        }
        let mut pcm = AlsaPcm { device, config, capture: None, playback: None, open_set: self.open_set.clone() };
        // On error `pcm` drops and clears the busy flag.
        if config.playback {
            let (s, period, periods) = Stream::open(self.card, device, true, self.rate, &config)?;
            pcm.config.period_frames = period;
            pcm.config.periods = periods;
            pcm.playback = Some(s);
        }
        if config.capture {
            let (s, period, periods) = Stream::open(self.card, device, false, self.rate, &config)?;
            pcm.config.period_frames = period;
            pcm.config.periods = periods;
            pcm.capture = Some(s);
        }
        Ok(Box::new(pcm))
    }

    fn mixer(&mut self) -> Result<Box<dyn Mixer>> {
        Ok(Box::new(AlsaMixer::open(self.card)?))
    }
}

// SAFETY: `SndCtlElemList` holds a raw pointer only transiently, and none
// of these types is shared; the trait objects need `Send`.
unsafe impl Send for AlsaMixer {}
