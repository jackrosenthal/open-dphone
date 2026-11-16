# Limits

## Running

| Limit | Value | At the limit |
| --- | --- | --- |
| Time for `App::start` | 5 seconds | The phone stops the app; the app does not start. |
| Time for one `App::handle` | 2 seconds | The phone stops the app without `Event::Exit`. |
| Linear memory | `memory_kib` of the manifest; 4 MiB by default, 16 MiB at most | An allocation past the limit fails; in a Rust app the failure is a trap, and the phone stops the app. |
| WebAssembly memories, tables, table elements, instances | 1, 4, 50,000, and 8 | The app does not start. |

The app's data (`include_bytes!`, `include_str!`) is part of the program and counts toward linear memory.  An app that the phone stopped at a limit starts again when opened.  An app that starts with the phone (`AppKind::Background`, or `has_idle`) is also started again by the phone, after a wait:

| Limit | Value | At the limit |
| --- | --- | --- |
| Wait before the phone starts a faulted `Background` or `has_idle` app again | 10 seconds after the first fault, twice the previous wait after each further fault in a row, 1 hour at most | The wait stays at 1 hour. |
| Time without a fault after which the count of faults starts over | 1 hour of running | The next wait is 10 seconds again.  Opening the app also starts the count over. |

## The app file

| Limit | Value | At the limit |
| --- | --- | --- |
| Size of the file | 8 MiB | Installation is refused. |
| Code in the file (the code sections; data does not count) | 1 MiB | Installation is refused. |
| Icon (`icon`, and each icon of `icon_for`) | 256 KiB | The crate does not compile; the phone refuses such a file. |
| App `name` | 1 to 64 characters | The crate does not compile; the phone refuses such a file. |
| Free flash after installation | 2 MiB | Installation is refused. |

## `ui` and `canvas`

| Limit | Value | At the limit |
| --- | --- | --- |
| Pictures defined by an app | 32 names | `define_image` returns `Error::Invalid`. |
| Size of a picture file | 512 KiB | `define_image` returns `Error::Invalid`. |
| Pixels of a picture | 1,048,576 | `define_image` returns `Error::Invalid`. |
| Characters typed into an input | 255, or the value of `set_max_length` (at most 65,535) | Further keys are ignored. |
| Softkeys shown at a time | 4 | A "More" key pages through the rest, three at a time. |
| Operations in one `canvas::draw` | 512 | `draw` returns `Err`; the canvas is unchanged. |
| Bitmap data in one `canvas::draw` | 64 KiB | `draw` returns `Err`; the canvas is unchanged. |
| Size of a bitmap | 256 by 256 pixels, scale 1 to 8 | `draw` returns `Err`; the canvas is unchanged. |
| Text of one canvas operation | 64 bytes | `draw` returns `Err`; `text_size` measures the first 64 bytes. |

## Timers

| Limit | Value | At the limit |
| --- | --- | --- |
| Timers of an app | 32 | `timers::set` returns `Error::Failed` and starts no timer. |
| Period of a repeating timer | 50 milliseconds at least | A shorter period runs at 50 milliseconds. |

## Storage

| Limit | Value | At the limit |
| --- | --- | --- |
| Value of a `storage` key | 1 MiB | `storage::set` returns `Error::Invalid`. |
| Length of a `storage` key | 64 bytes | `storage::set` and `storage::remove` return `Error::Invalid`; `storage::get` returns `None`. |
| Free flash kept by the phone | 2 MiB | `storage::set` returns `Error::NoSpace`. |
| Key of `pbx_storage` | 1 to 64 characters of letters, digits, `.`, `_`, `-`, `/`, `:` | The request ends `Refused`. |
| Value of a `pbx_storage` key | 4096 bytes | The request ends `Refused`. |
| Keys of `pbx_storage` | 1024 shared, and 64 for each user | The request ends `Refused`. |
| Entries of one `pbx_storage::list` answer | 64 entries and about 16 KiB of values | The answer says that more follow. |

## Network

| Limit | Value | At the limit |
| --- | --- | --- |
| HTTP response, head and body | 1 MiB | `Event::Http` carries `Err("reply too large")`. |
| HTTP connection | 10 seconds | `Event::Http` carries `Err`. |
| HTTP request and response | 30 seconds | `Event::Http` carries `Err("the request timed out")`. |
| HTTP redirects | 5 | `Event::Http` carries `Err("too many redirects")`. |

## Calls, presence, and events

| Limit | Value | At the limit |
| --- | --- | --- |
| Number given to `calls::dial` | 256 bytes | `Error::Invalid`. |
| Extra headers of `calls::dial` | 30; names up to 64 characters, values up to 512 bytes | `Error::Invalid`. |
| Digits of one `calls::send_dtmf` | 64 | `Error::Invalid`. |
| Calls of the phone | 4 unless configured otherwise | `calls::dial` returns `Error::Invalid`. |
| Answer from call control, presence, or provisioning | 0.5 seconds | `Error::Failed`; the request may still be carried out. |
| Targets of `presence::observe` | 256 | `observe` returns `Error::Failed`. |
| Contacts the phone watches | The setting `contacts_max_subscriptions` of the phone's configuration, 40 by default | The phone watches no further contact, and `presence::get` returns `None` for such a contact. |
| Names of `events::observe` | 64 | `observe` returns `Error::Failed`. |
| Extension and PIN of `hotdesk::login` | 64 bytes each | `Error::Invalid`. |

## Audio, alarms, and the log

| Limit | Value | At the limit |
| --- | --- | --- |
| Sound given to `audio::play` | 8 MiB and five minutes | `Error::Failed`. |
| Sounds playing | 1 for the whole phone | A new sound replaces the one playing. |
| Alarms | 16 | `alarms::set` returns `Error::Invalid`. |
| Answer from the alarm clock | 1.5 seconds | `Error::Failed`; the request may still be carried out. |
| Message of `runtime::log` | 512 bytes | The message is cut. |
| Entries of `diagnostics::log_tail` | 100 | The list is shorter than asked. |
