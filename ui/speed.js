// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * speed.js
 *
 * Version: @(#)speed.js 0.3.0 2026
 *
 * Description: The Lab Notebook.
 *              Reference numbers from one rig, so you know CPU vs GPU and
 *              model vs model before you download 3 GB. Sentences: eight
 *              phrases of different lengths, p50 of
 *              `bench --first 1.8,3.1,…,5.2 [--gap 5000 --press 2000] --json`
 *              (a repeated slice flatters DirectML; real phrases never repeat
 *              a length). Long clip + WER: `race` on both built-in clips.
 *              Measured, never typed in. Regenerate after any runtime bump.
 *
 * Author: Magnus Leatti (layer07)
 * URL: https://kernelriot.com
 * Github: /layer07
 */
window.DV_SPEED = {
 "rig": {
  "cpu": "Intel Core i9-14900KF",
  "gpu": "RTX 3080 10 GB · DirectML",
  "ram": "64 GB",
  "threads": 8,
  "when": "2026-10-28 · NVIDIA 566.36 · parakeet-rs 0.4.3",
  "note": "Measured with Parsec streaming the desktop and a browser open: how this machine is actually used. Sentences swing ±30–50% run to run; repeated passes are averaged. The ranking holds."
 },
 "rows": [
  {
   "id": "parakeet-unified-int8",
   "name": "Parakeet Unified · int8",
   "engine": "unified",
   "wer": [
    3.4,
    3.8
   ],
   "dev": {
    "cpu": {
     "idle": 196,
     "warm": 83,
     "cold": 64,
     "load": 1930,
     "long": 884,
     "long_xrt": 45.0
    },
    "directml": {
     "idle": 242,
     "warm": 147,
     "cold": 144,
     "load": 2468,
     "long": 1561,
     "long_xrt": 25.5
    }
   }
  },
  {
   "id": "parakeet-ultra",
   "name": "Parakeet Ultra",
   "engine": "tdt",
   "wer": [
    3.4,
    2.6
   ],
   "dev": {
    "cpu": {
     "idle": 120,
     "warm": 113,
     "cold": 83,
     "load": 2461,
     "long": 1360,
     "long_xrt": 29.2
    },
    "directml": {
     "idle": 242,
     "warm": 161,
     "cold": 132,
     "load": 2012,
     "long": 1235,
     "long_xrt": 32.2
    }
   }
  },
  {
   "id": "parakeet-unified",
   "name": "Parakeet Unified",
   "engine": "unified",
   "wer": [
    3.4,
    3.8
   ],
   "dev": {
    "cpu": {
     "idle": 212,
     "warm": 100,
     "cold": 71,
     "load": 2920,
     "long": 1024,
     "long_xrt": 38.8
    },
    "directml": {
     "idle": 282,
     "warm": 216,
     "cold": 224,
     "load": 2175,
     "long": 1986,
     "long_xrt": 20.0
    }
   }
  },
  {
   "id": "parakeet-v3",
   "name": "Parakeet v3",
   "engine": "tdt",
   "wer": [
    3.4,
    2.6
   ],
   "dev": {
    "cpu": {
     "idle": 94,
     "warm": 88,
     "cold": 71,
     "load": 2621,
     "long": 997,
     "long_xrt": 39.9
    },
    "directml": {
     "idle": 76,
     "warm": 41,
     "cold": 57,
     "load": 2360,
     "long": 275,
     "long_xrt": 144.7
    }
   }
  },
  {
   "id": "cohere-transcribe",
   "name": "Cohere Transcribe · int8",
   "engine": "cohere",
   "wer": [
    1.7,
    2.6
   ],
   "dev": {
    "cpu": {
     "idle": 3828,
     "warm": 4447,
     "cold": 2604,
     "load": 1209,
     "long": 19304,
     "long_xrt": 2.1
    },
    "directml": {
     "error": "Cohere Transcribe doesn't run on DirectML: ONNX Runtime's DirectML attention kernel rejects its decoder. Use CPU or CUDA"
    }
   }
  },
  {
   "id": "nemotron-3.5",
   "name": "Nemotron 3.5",
   "engine": "nemotron",
   "wer": [
    1.7,
    2.6
   ],
   "dev": {
    "cpu": {
     "idle": 499,
     "warm": 389,
     "cold": 253,
     "load": 1941,
     "long": 4861,
     "long_xrt": 8.2
    },
    "directml": {
     "idle": 1435,
     "warm": 686,
     "cold": 396,
     "load": 1727,
     "long": 7504,
     "long_xrt": 5.3
    }
   }
  },
  {
   "id": "nemotron-en",
   "name": "Nemotron Streaming · English",
   "engine": "nemotron",
   "wer": [
    1.7,
    3.8
   ],
   "dev": {
    "cpu": {
     "idle": 805,
     "warm": 466,
     "cold": 299,
     "load": 1989,
     "long": 5116,
     "long_xrt": 7.8
    },
    "directml": {
     "idle": 958,
     "warm": 666,
     "cold": 410,
     "load": 1726,
     "long": 6884,
     "long_xrt": 5.8
    }
   }
  }
 ]
};
