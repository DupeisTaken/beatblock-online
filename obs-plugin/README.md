# Beatblock Together OBS plugin

This native OBS 32.1.x plugin registers the stable **Beatblock Together Player Stream** source for Manager slots A-D and the **Beatblock Together Shared Audio** source contract.

Configure a build against an OBS SDK that exports `OBS::libobs`:

```text
cmake -S obs-plugin -B obs-plugin/build -Dlibobs_DIR=<obs-sdk>/cmake
cmake --build obs-plugin/build --config Release
```

The Manager installer embeds and installs the resulting `beatblock-together-obs.dll` when `BBT_OBS_PLUGIN_DLL` points to it during the Manager build. Without an OBS SDK build artifact, the Manager clearly reports that the optional source is unavailable.

Video frame ingestion is implemented through the Manager's versioned shared frame ring. Process-specific audio capture and song-only fallback remain a certification gate; the current alpha audio source registers the contract but intentionally emits no misleading audio until that path is built and verified against OBS 32.1.2.
