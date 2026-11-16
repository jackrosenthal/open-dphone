# Logs and diagnostics

When a change misbehaves on a phone, the first questions are what the firmware logged and what state the firmware is in.  This page shows where each kind of log is kept, how long each survives, and which tools show the state of a running phone.

## Where the logs are

Three parts of the firmware log to three places:

| Source | Where the log goes | Survives |
| --- | --- | --- |
| The main role | A ring in the main role's memory, of the last 2,000 entries | Until the main role restarts |
| The init role | The kernel's log | Until the phone restarts |
| The kernel | The kernel's log; the last 8 KiB also in memory that keeps the contents across a reset | A reset, as a crash record |

### The main role's log

```sh
dphone log            # the last 200 entries
dphone log -n 50 -f   # the last 50, then follow
```

Each entry has the time, the level, the module that logged, and the message.  The **Logs** page of the web interface shows the same ring live, with a filter by level and by text.

On a phone, the main role logs at the information level and above.  The simulator logs at the debug level when started with `--verbose`.

The ring is lost when the main role restarts.  To keep a log across a failure, follow the log from the workstation into a file:

```sh
ssh root@192.0.2.20 dphone log -f | tee phone.log
```

The phone can also send every entry to a syslog server.  On the **System** page, the **Logging** card has **Remote syslog**, the server, the port, and the level that the server receives.

While the [serial console](debug-access.md#the-serial-console-on-the-ehs-port) has the EHS port, the main role's log is copied to the console as well.

### The init role and the kernel

```sh
dmesg
```

The init role writes to the kernel's log, so `dmesg` shows the kernel's messages and the init role's decisions in one sequence: the modules loaded, each start and failure of the main role, a start in safe mode, the trial boot, and a return to the previous bank.  When the main role does not stay up, `dmesg` is the place to look, because the main role's ring is gone with the failed process.

### Crash records

The kernel keeps the last 8 KiB of console output, and the last 8 KiB of the kernel log from a panic, in memory that survives a reset.  At the next start, the main role moves those records to `/nvdata/open/crash/`, where the records replace the records of the crash before.

```sh
dphone crashlog
```

The command prints the saved records.  After an unexplained restart, the records show what the kernel printed last.

## The state of a running phone

### Is the phone healthy?

```sh
dphone health
```

The command lists the conditions that the main role requires before reporting the phone healthy to the init role, and whether each holds.  The same report confirms a [trial boot](installing.md#the-trial-boot), so `dphone health` answers why a trial is not confirmed.  A subsystem whose loop has stopped is shown as degraded here, in the web interface, and in the log.

### What does a subsystem believe?

```sh
dphone state
dphone state audio
```

Every subsystem publishes state to one typed store, and the user interface, the web interface, and the commands read from that store.  `dphone state` dumps the store, and a word limits the dump to the top-level fields whose name contains the word.  When the screen and the web interface disagree with what the phone is doing, the dump shows whether the state or the display is wrong.

The subsystem commands (`dphone net`, `dphone audio`, `dphone dsp`, `dphone ehs`, `dphone hal`, and others) show more detail of one subsystem; see [The `dphone` command](dphone-command.md#hardware-and-subsystems).

### What does the configuration say?

```sh
dphone config list sip
dphone config get log_level
```

`dphone config` reads the merged configuration, i.e., the result of every configuration source that the phone combines.  Secrets are masked in the list.

## The web interface for a developer

The web interface is at the phone's address, over HTTP and HTTPS.  Four pages serve development directly:

| Page | Use |
| --- | --- |
| **Logs** | The main role's log, live, with filters. |
| **Screen** | A live mirror of the phone's display, with the phone's keys to select.  A developer can operate a phone on the other side of the room, or a phone whose screen is out of sight. |
| **Shell** | A root shell in the browser, with the same shell and commands as an SSH session.  The page needs no SSH key and no SSH setting. |
| **System** | The firmware card ([Installing a firmware image](installing.md#with-the-web-interface)), and the SSH and EHS port settings ([SSH and the serial console](debug-access.md)). |

## Metrics

`GET /metrics` on the phone returns metrics in the Prometheus text format, with no login.  The metrics cover the build, the uptime, the health of each subsystem, the network link, the registration of each account, the calls, the RTP totals, provisioning, the firmware bank and the trial state, flash error counters, memory, and load.  A scrape of a phone under test over hours shows trends that a log does not, e.g., memory growth.

```sh
curl http://192.0.2.20/metrics
```

The endpoint is on unless switched off, with **Prometheus metrics** in the **Logging** card of the **System** page or with [`dphone_metrics_enabled=no`](../open-dpma/open-dphone-options.md#dphone_metrics_enabled) from the PBX.
