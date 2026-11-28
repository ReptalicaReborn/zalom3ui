# Performance monitor

A debug overlay and a log for testing Zalỏ on slow phones, like the debug overlay in Discord. It measures how smoothly the app draws, how much memory and CPU it uses, and how busy the link to the Zalo core is. It holds numbers only.

## Turn it on

Open Settings → Developer. (Developer shows only while Zalo runs on this phone and you are logged in.)

- **Performance monitor** shows the overlay on top of every screen.
- **Log performance** writes `perf.log`. It works with or without the overlay.
- **Share performance log** sends the newest part of `perf.log` (and the phone and app version) to another app.
- **Clear performance log** empties it.

Both switches are off by default. When both are off, nothing is running: no listener, no thread. The only cost that is always there is a few counters on each call to the core (two clock reads and some additions).

The monitor runs only while Zalỏ is on screen. It stops when you leave the app and starts again when you come back, so it never measures (or drains) the app in the background.

## The overlay

A translucent card in the top right corner.

- Drag it anywhere. It stays where you leave it until the app process ends.
- Tap its title to fold it to one line: `58fps 9.1ms · 312MB · 14%` (frames per second, average frame time, PSS, CPU). Tap again to unfold.
- The × turns the setting off.
- Only the card takes touches. The rest of the screen works as usual.
- Full-screen dialogs, such as the video player, are separate windows above the app, so the card is not visible there.

Each section has a small graph of the last 30 samples (15 seconds), newest on the right. Bars are green, amber or red:

| Graph | Shows | Green / amber / red |
| --- | --- | --- |
| FRAME | the longest frame of each half second | within the budget / up to twice it / more |
| MEMORY | PSS, scaled between its lowest and highest value | enough free RAM / under twice Android's low-memory threshold / low memory |
| CPU | CPU of one core | under 60% / under 100% / 100% or more |
| CORE | average latency of a call to the core | under 50 ms / under 200 ms / more |

The overlay updates twice a second.

## What the numbers mean

### FRAME

Taken from `FrameMetrics` of the app window, so only frames that were really drawn count. An app that draws nothing for a second shows 0 fps, which is fine: it is idle, not slow.

- **fps**: frames drawn per second. It is not the refresh rate. A static screen draws nothing. Android can report the same frame more than once for one vsync; those repeats are not counted, so fps never goes above what the display can show. If it happens, the log line gets a ` dup=N` key (repeats in that window), placed after `slow=`; the key is absent when there were none.
- **avg / max**: average and longest time to produce a frame in the sample (the total duration Android measures: input, layout, draw and the GPU hand-off).
- **jank**: the share of frames in the sample that took longer than their frame budget, in percent. On Android 12 and newer the budget is the deadline Android gives each frame (it follows an adaptive display as it changes rate). Otherwise, or if that value looks wrong, it is one refresh: 16.7 ms at 60 Hz, 8.3 ms at 120 Hz (read from the display, 16.7 ms if unknown). Jank 0% means every frame made its refresh. Jank above about 5% is visible as stutter; above 20% is a bad stutter.
- **slow**: the number of those frames.
- **worst**: the longest frame in the last 30 seconds. It stays after the stutter is over, so you can look away and still see it.

The top right of the section says the refresh rate and the budget in use. Phones that change their refresh rate on their own (adaptive 10 to 120 Hz) report the current mode, not what the display really does at that moment; the refresh-rate budget is therefore only an approximation there, which is why Android 12 and newer use each frame's own deadline.

### MEMORY

Four numbers that measure different things. Do not add them up.

- **pss**: Proportional Set Size from `Debug.getMemoryInfo`. All the app's memory, with memory shared with other processes (system libraries, the graphics driver) divided among them. This is what Android's own tools show and the best single number to compare between phones. It is slowish to read, so it updates every 2 seconds.
- **rss**: Resident Set Size, from `/proc/self/status`. All the app's pages that are in RAM, shared pages counted in full. It is larger than PSS.
- **peak**: the highest RSS the process has reached (`VmHWM`).
- **java**: the Java/Kotlin heap in use (`Runtime`). It is limited per app by Android and is what the garbage collector works on.
- **native**: memory allocated by native code (`Debug.getNativeHeapAllocatedSize`). The Rust Zalo core runs inside this process and uses the system allocator, so its memory is here, together with bitmaps and codecs.
- **gc**: garbage collections per second (`art.gc.gc-count`). A few per second while scrolling is normal. Five or more per second for a long time means the app is allocating too much (the log marks it `GC-HEAVY` from 4 per second). Shows `n/a` if the phone does not report it.

DEVICE shows the RAM that is free on the whole phone, out of the total. When Android says the phone is short of memory, a red **LOW MEM** appears (and `LOWMEM` goes to the log). Below that, Android's own limit is the "threshold": under about twice that the card turns the graph amber.

### CPU

**cpu** is the CPU time the whole process used, as a percent of **one core**. 100% is one core busy all the time. A phone with 8 cores can show up to 800%. It comes from `/proc/self/stat`, which counts in 10 ms steps, so a half-second sample is accurate to about 2 points. **thr** is the number of threads.

### CORE (the bridge to the Zalo core)

Counts every request the app makes to the Zalo core, whether it goes in-process (JNI, the "this phone" mode) or over the network to a Zalo server. It is the equivalent of Discord's BRIDGE section.

- **req/s**: requests finished per second.
- **avg / max**: how long a request took, from the call to the answer, in milliseconds. This includes waiting for the core to start. Uploads (photos, files, voice) are counted as requests but left out of the latency, because they last as long as the file takes to send.
- **inflight**: requests that have started and not finished.
- **ev/s**: events from the core per second (new message, typing, status): what the live connection delivers.
- **io**: kilobytes per second sent to and received from the core (`up/down`). It counts characters of the answers and events, not exact bytes, and does not count voice files sent. Use it to compare, not to bill.
- **errors**: shown in red in the section title when requests failed with an exception (no answer at all). A 4xx or 5xx answer is an answer, not an error.

High **max** with low **req/s** usually means one slow call. High **inflight** that does not go down means calls are queuing.

### DEVICE

Free and total RAM, thermal status (`none`, `light`, `moderate`, `severe`, `critical`, `emergency`, `shutdown`; Android 10 and later) and battery temperature, and the screen the app is on (`home`, `chat`, `settings`, ...: the navigation route).

## perf.log

The file is `files/zalo/perf.log` inside the app's private storage, next to `startup.log`. It is not readable by other apps; use **Share performance log** to get it out. It keeps at most 256 KB; when it passes that, only the newest half is kept. That is about 20 to 40 minutes of lines.

Each time logging starts (the switch is turned on, or the app process starts with it on) the log gets a header: a line with the date and process id, then the app version and build type, the phone (manufacturer, model, hardware, Android version, ABIs), total RAM, core count, refresh rate and a short list of the fields. A header makes two logs from two phones comparable.

While Zalỏ is on screen and logging is on, a line is written every 2 seconds:

```
14:03:22.101 fps=58 hz=120 frame=9.1/22ms jank=3% slow=2 cpu=12% pss=312MB rss=401MB java=48MB native=120MB avail=1.1GB core=14req/s lat=3.2/18ms inflight=0 ev=3.2/s io=1/34KB/s gc=0.0 thr=46 therm=0 temp=31.2 screen=chat
```

Every key is always there, in this order, as `key=value`, so the log can be grepped and plotted. Unknown values read `-1`. `hz` is the display's refresh rate at that moment, so a log shows when it dropped (the app never asks for a rate itself, so a change is the system's doing). `frame` and `lat` are `average/maximum`, `io` is `up/down`, `therm` is Android's thermal status number (0 is none, 2 is moderate, 3 is severe), `temp` is the battery temperature in degrees Celsius. The line covers the 2 seconds since the previous one: fps, jank and the core numbers are averages (and maxima) over that time.

Other lines mark events:

| Line | When |
| --- | --- |
| `FG` / `BG` | the app came on screen / left it |
| `JANK 142ms screen=chat` | a single frame took 100 ms or more (at most one per second) |
| `HZ 120->90 screen=chat` | the display's refresh rate changed (as `Display.refreshRate` reports it: on an adaptive display, the current mode), with the screen the app was on; checked every 2 seconds |
| `TRIM RUNNING_LOW` | Android asked the app to free memory (`onTrimMemory`); the level is named. This also logs while the app is in the background, if the process is alive. |
| `LOWMEM avail=… threshold=…` | Android says the phone is low on memory (at most one per 30 seconds) |
| `GC-HEAVY gc=6.5/s java=48MB` | 4 or more garbage collections per second (at most one per 10 seconds) |

Nothing in the log is a message, a name, a phone number or a token. It holds counts, times, sizes and the name of the screen.

## Comparing phones

Run the same steps on each phone (for example: open the chat list, open a busy group, scroll up through 200 messages, send a photo), with the overlay or the log on, and compare:

- **jank** and **max** while scrolling: the main measure of how smooth it feels. Compare phones at the same refresh rate (or compare `frame=` in ms).
- **pss** and **native** after the same steps: how much memory the app needs. Phones with 2 to 3 GB of RAM start to show `LOWMEM` and `TRIM` lines when the number is too high.
- **avail** next to **pss**: whether the phone has room left.
- **cpu** while idle and while scrolling. High idle CPU is a bug worth reporting.
- **lat** in CORE: how long the core takes to answer. High on a slow phone and low on a fast one means the core is the bottleneck; high everywhere means Zalo or the network is.
- **gc** and **java** growing without coming back: a leak or too many allocations.
- **therm** rising during a long run: the phone slows down by itself when it is hot, so a late stutter may be the heat.

## What it costs

The monitor costs a little itself. Reading PSS walks the app's memory map and can take tens of milliseconds on a slow phone; it runs every 2 seconds on a background thread, never on the main thread. The overlay redraws a small card twice a second. When you compare phones, compare them with the monitor on for all of them. For the cleanest frame numbers, use the log alone (one sample per 2 seconds, no overlay).

The 32-bit build and the 64-bit build use the same monitor.
