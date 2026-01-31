# Audio Bridge

```bash
src/
  main.rs        // orchestration only
  cli.rs         // Args + parsing
  device.rs      // list/pick output device
  queue.rs       // SharedAudio bounded queue + condvar
  decode.rs      // Symphonia streaming decode thread
  resample.rs    // Rubato resampler thread
  playback.rs    // CPAL output stream + channel mapping
```

## Options

* Default (USB stable):

    `--buffer-seconds 2.0 --chunk-frames 1024 --refill-max-frames 4096`

* Paranoid stable (recording session / heavy CPU):

    `--buffer-seconds 4.0 --chunk-frames 2048 --refill-max-frames 8192`

* Lower latency (if you want snappier stop/start):

    `--buffer-seconds 0.75 --chunk-frames 512 --refill-max-frames 2048`