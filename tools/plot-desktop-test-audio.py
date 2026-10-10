"""Plot a verified synthetic VM continuous recording; never open audio devices.

Requires numpy and matplotlib. Usage: python tools/plot-desktop-test-audio.py PATH
where PATH is an imported VM evidence directory.
"""
import hashlib
import json
from pathlib import Path
import struct
import sys

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


def main(folder: Path) -> None:
    metadata = json.loads((folder / "continuous-a-to-b.json").read_text(encoding="utf-8"))
    raw = (folder / "continuous-a-to-b.wav").read_bytes()
    if hashlib.sha256(raw).hexdigest() != metadata["wav_sha256"]:
        raise ValueError("Recording SHA256 mismatch")
    tag, channels, rate = struct.unpack_from("<HHI", raw, 20)
    if raw[:4] != b"RIFF" or raw[8:12] != b"WAVE" or tag != 3 or channels != 1:
        raise ValueError("Expected the probe's mono float WAV")
    samples = np.frombuffer(raw, dtype="<f4", offset=44).astype(np.float64)
    if not np.isfinite(samples).all():
        raise ValueError("Nonfinite recording")
    width = rate // 10
    step = rate // 20
    window = np.hanning(width)
    index = np.arange(width)
    times, a, b = [], [], []
    for start in range(0, len(samples) - width + 1, step):
        section = samples[start:start + width] * window
        times.append((start + width / 2) / rate)
        a.append(2 * abs(np.dot(section, np.exp(-2j * np.pi * 440 * index / rate))) / window.sum())
        b.append(2 * abs(np.dot(section, np.exp(-2j * np.pi * 880 * index / rate))) / window.sum())
    measurement = metadata["measurement"]
    switch = measurement["switch_frame"] / rate
    settled = switch + measurement["settling_frames"] / rate
    fig, ax = plt.subplots(figsize=(10, 4.6), layout="constrained")
    ax.plot(times, a, color="#e05252", linewidth=2, label="A: 440 Hz")
    ax.plot(times, b, color="#2474d2", linewidth=2, label="B: 880 Hz")
    ax.axvspan(0, .25, color="#ddd", alpha=.6, label="Capture startup (250 ms)")
    ax.axvspan(switch, settled, color="#ffd47a", alpha=.35, label="1 s settling bound")
    ax.axvline(switch, color="#333", linestyle="--", linewidth=1)
    ax.axvline(settled, color="#4c8d52", linestyle=":", linewidth=1)
    ax.set(title="C loopback: uninterrupted recording across default A → B",
           xlabel="Recorded sample time (seconds)", ylabel="Tone amplitude", ylim=(-.005, .17))
    ax.grid(alpha=.2)
    ax.legend(loc="upper right", framealpha=.95, fontsize=9)
    fig.savefig(folder / "continuous-a-to-b.png", dpi=180)
    fig.savefig(folder / "continuous-a-to-b.svg")
    plt.close(fig)
    print(folder / "continuous-a-to-b.png")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
