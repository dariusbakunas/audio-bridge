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