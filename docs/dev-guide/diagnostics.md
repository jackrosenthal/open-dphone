# Logs and diagnostics

## Log locations

| Source | Destination | Retention |
| --- | --- | --- |
| Main role | A ring buffer in the main role's memory that holds the last 2,000 entries or 256 KiB | Until the main role restarts |
| Init role | The kernel log | Until the phone restarts |
| Kernel | The kernel log.  The last 8 KiB of console output, and the last 8 KiB of the log at a panic, are also kept in memory that is preserved across a reset. | Across a reset, but not a power loss |

## Main role log

```sh
dphone log            # the last 200 entries
dphone log -n 50 -f   # the last 50, then follow
```

Each entry has the time, the level, the module that logged the entry, and the message.  The **Logs** page of the web interface shows the same ring buffer live, with filters for level and text.

On a phone, the main role logs at the info level and above.  The simulator logs at the debug level when started with `--verbose`.

The ring buffer is lost when the main role restarts.  To keep a log across a failure, follow the log from the workstation and save the output to a file:

```sh
ssh root@192.0.2.20 dphone log -f | tee phone.log
```

The phone can also send every entry to a syslog server.  On the **System** page, the **Logging** card has the **Remote syslog** switch, the server, the port, and the minimum level to send.

While the [serial console](ssh-and-serial-console.md#serial-console) has the EHS port, the main role's log is also copied to the console.

## Kernel log

```sh
dmesg
```

The init role writes to the kernel log, so `dmesg` shows the kernel's messages and the init role's decisions in one sequence: the modules loaded, each start and failure of the main role, a start in safe mode, the trial boot, and a rollback to the previous bank.  When the main role does not stay up, check `dmesg` first, because the main role's ring buffer disappears with the failed process.

## Crash records

The kernel keeps two records in memory that is preserved across a reset:

- **The panic log.**  After a kernel panic, the main role moves the last 8 KiB of the kernel log to `/nvdata/open/crash/` at the next start, replacing the records from any earlier panic.  `dphone crashlog` prints the saved records, and the main role logs a message at startup when records are found.
- **The console output.**  The last 8 KiB of the kernel's console output before any restart that does not involve a power loss, e.g., a watchdog reset, is in `/sys/fs/pstore/console-ramoops-0` until the next restart.  After an unexplained restart, check that file to see what the kernel printed last.

```sh
dphone crashlog
cat /sys/fs/pstore/console-ramoops-0
```

## Runtime state

### Health

```sh
dphone health
```

The command lists the conditions that the main role requires before reporting the phone as healthy to the init role, and whether each condition is met.  The same report confirms a [trial boot](firmware-updates.md#trial-boot), so `dphone health` explains why a trial has not been confirmed.  A subsystem whose loop has stopped is shown as degraded here, in the web interface, and in the log.

### State store

```sh
dphone state
dphone state audio
```

Every subsystem publishes state to a single typed store, and the user interface, the web interface, and the commands all read from that store.  `dphone state` dumps the store, and an argument limits the dump to the top-level fields whose names contain the word.  When the screen or the web interface disagrees with what the phone is doing, the dump shows whether the state or the display is wrong.

The [subsystem commands](dphone-command.md#hardware-and-subsystems), such as `dphone net`, `dphone audio`, `dphone dsp`, `dphone ehs`, and `dphone hal`, show more detail about one subsystem.

### Configuration

```sh
dphone config list sip
dphone config get log_level
```

`dphone config` reads the merged configuration, i.e., the combined result of all of the phone's configuration sources.  Secrets are masked in the list.

## Web interface

The web interface is available at the phone's address over HTTP and HTTPS.  Four pages are especially useful for development:

| Page | Description |
| --- | --- |
| **Logs** | The main role's log, live, with filters. |
| **Screen** | A live mirror of the phone's display, with clickable phone keys.  Use the page to operate a phone that is across the room or out of sight. |
| **Shell** | A root shell in the browser, with the same shell and commands as an SSH session.  The page works without an SSH key and with the SSH setting off. |
| **System** | The [firmware card](firmware-updates.md#web-interface), and the [SSH and EHS port settings](ssh-and-serial-console.md). |

## Metrics

`GET /metrics` on the phone returns metrics in the Prometheus text format, with no login required.  The metrics cover the build, the uptime, the health of each subsystem, the network link, the registration state of each account, the calls, the RTP totals, provisioning, the firmware bank and the trial state, flash error counters, memory, and load.  Scraping a phone under test for several hours reveals trends that a log does not, e.g., memory growth.

```sh
curl http://192.0.2.20/metrics
```

The endpoint is on by default.  To turn the endpoint off, use the **Prometheus metrics** switch in the **Logging** card of the **System** page, or set [`dphone_metrics_enabled=no`](../open-dpma/open-dphone-options.md#dphone_metrics_enabled) on the PBX.
