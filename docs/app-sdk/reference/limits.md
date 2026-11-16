# Limits

## Execution

| Limit | Value | When reached |
| --- | --- | --- |
| Run time of `App::start` | 5 seconds | The phone stops the app, and the app doesn't start. |
| Run time of one `App::handle` call | 2 seconds | The phone stops the app without sending `Event::Exit`. |
| Linear memory | `memory_kib` in the manifest: 4 MiB by default, 16 MiB at most | An allocation beyond the limit fails.  In a Rust app, the failure is a trap, and the phone stops the app. |
| WebAssembly memories, tables, table elements, and instances | 1, 4, 50,000, and 8 | The app doesn't start. |

Embedded data (`include_bytes!`, `include_str!`) is part of the program and counts toward linear memory.  An app that was stopped at a limit starts again when opened.  The phone also restarts an app that starts with the phone (`AppKind::Background`, or `has_idle`), after a delay:

| Limit | Value | When reached |
| --- | --- | --- |
| Delay before a faulted `Background` or `has_idle` app restarts | 10 seconds after the first fault, doubling with each consecutive fault, up to 1 hour | The delay stays at 1 hour. |
| Fault-free run time that resets the count of consecutive faults | 1 hour | The next delay is 10 seconds again.  Opening the app also resets the count. |

## App file

| Limit | Value | When reached |
| --- | --- | --- |
| File size | 8 MiB | Installation fails. |
| Code size (the code sections; data doesn't count) | 1 MiB | Installation fails. |
| Icon size (`icon`, and each icon in `icon_for`) | 256 KiB | The crate doesn't compile, and the phone rejects such a file. |
| Length of the app `name` | 1 to 64 characters | The crate doesn't compile, and the phone rejects such a file. |
| Free flash after installation | 2 MiB | Installation fails. |

## `ui` and `canvas`

| Limit | Value | When reached |
| --- | --- | --- |
| Images defined by an app | 32 names | `define_image` returns `Error::Invalid`. |
| Image file size | 512 KiB | `define_image` returns `Error::Invalid`. |
| Image pixel count | 1,048,576 | `define_image` returns `Error::Invalid`. |
| Characters typed into an input | 255, or the value passed to `set_max_length` (at most 65,535) | Further key presses are ignored. |
| Softkeys shown at once | 4 | A "More" key pages through the rest, three at a time. |
| Operations in one `canvas::draw` call | 512 | `draw` returns `Err`, and the canvas is unchanged. |
| Bitmap data in one `canvas::draw` call | 64 KiB | `draw` returns `Err`, and the canvas is unchanged. |
| Bitmap dimensions | 256 by 256 pixels, scale 1 to 8 | `draw` returns `Err`, and the canvas is unchanged. |
| Text in one canvas operation | 64 bytes | `draw` returns `Err`.  `text_size` measures the first 64 bytes. |

## Timers

| Limit | Value | When reached |
| --- | --- | --- |
| Timers per app | 32 | `timers::set` returns `Error::Failed` and starts no timer. |
| Minimum period of a repeating timer | 50 milliseconds | A shorter period runs at 50 milliseconds. |

## Storage

| Limit | Value | When reached |
| --- | --- | --- |
| `storage` value size | 1 MiB | `storage::set` returns `Error::Invalid`. |
| `storage` key length | 64 bytes | `storage::set` and `storage::remove` return `Error::Invalid`, and `storage::get` returns `None`. |
| Free flash reserved by the phone | 2 MiB | `storage::set` returns `Error::NoSpace`. |
| `pbx_storage` key | 1 to 64 characters: letters, digits, `.`, `_`, `-`, `/`, and `:` | The request ends in `Refused`. |
| `pbx_storage` value size | 4096 bytes | The request ends in `Refused`. |
| `pbx_storage` key count | 1024 shared, and 64 per user | The request ends in `Refused`. |
| Entries in one `pbx_storage::list` answer | 64 entries and about 16 KiB of values | The answer indicates that more entries follow. |

## Network

| Limit | Value | When reached |
| --- | --- | --- |
| HTTP response size, headers and body combined | 1 MiB | `Event::Http` carries `Err("reply too large")`. |
| HTTP connection time | 10 seconds | `Event::Http` carries `Err`. |
| HTTP request and response time | 30 seconds | `Event::Http` carries `Err("the request timed out")`. |
| HTTP redirects | 5 | `Event::Http` carries `Err("too many redirects")`. |

## Calls, presence, and events

| Limit | Value | When reached |
| --- | --- | --- |
| Length of the number passed to `calls::dial` | 256 bytes | `Error::Invalid`. |
| Extra headers passed to `calls::dial` | 30, with names up to 64 characters and values up to 512 bytes | `Error::Invalid`. |
| Digits in one `calls::send_dtmf` call | 64 | `Error::Invalid`. |
| Simultaneous calls on the phone | 4 unless configured otherwise | `calls::dial` returns `Error::Invalid`. |
| Wait for an answer from call control, presence, or provisioning | 0.5 seconds | `Error::Failed`.  The request may still be carried out. |
| Targets observed with `presence::observe` | 256 | `observe` returns `Error::Failed`. |
| Contacts that the phone watches | The `contacts_max_subscriptions` setting in the phone's configuration, 40 by default | The phone watches no additional contacts, and `presence::get` returns `None` for an unwatched contact. |
| Names observed with `events::observe` | 64 | `observe` returns `Error::Failed`. |
| Length of the extension and PIN passed to `hotdesk::login` | 64 bytes each | `Error::Invalid`. |

## Audio, alarms, and logging

| Limit | Value | When reached |
| --- | --- | --- |
| Sound passed to `audio::play` | 8 MiB and five minutes | `Error::Failed`. |
| Sounds playing at once | 1 for the whole phone | A new sound replaces the one that is playing. |
| Alarms | 16 | `alarms::set` returns `Error::Invalid`. |
| Wait for an answer from the alarm clock | 1.5 seconds | `Error::Failed`.  The request may still be carried out. |
| Length of a `runtime::log` message | 512 bytes | The message is truncated. |
| Entries returned by `diagnostics::log_tail` | 100 | The list is shorter than requested. |
