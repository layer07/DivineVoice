// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * speed.js
 *
 * Version: @(#)speed.js 0.2.0 2026
 *
 * Description: The Lab Notebook.
 *              Reference numbers from one rig, so you know CPU vs GPU and
 *              model vs model before you download 3 GB. Sentence numbers:
 *              `bench --first 5 [--gap 5000] --json`, p50. Long clip + WER:
 *              `race` on both built-in clips. Measured, never typed in.
 *              Regenerate after a model or runtime bump.
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
  "when": "2026-10-28 · NVIDIA 566.36 · parakeet-rs 0.4.3"
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
     "idle": 123,
     "warm": 114,
     "cold": 126,
     "load": 1985,
     "long": 884,
     "long_xrt": 45.0
    },
    "directml": {
     "idle": 291,
     "warm": 135,
     "cold": 1095,
     "load": 2438,
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
     "idle": 130,
     "warm": 132,
     "cold": 143,
     "load": 2569,
     "long": 1360,
     "long_xrt": 29.2
    },
    "directml": {
     "idle": 246,
     "warm": 176,
     "cold": 527,
     "load": 2269,
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
     "idle": 144,
     "warm": 137,
     "cold": 150,
     "load": 2439,
     "long": 1024,
     "long_xrt": 38.8
    },
    "directml": {
     "idle": 336,
     "warm": 264,
     "cold": 573,
     "load": 2323,
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
     "idle": 148,
     "warm": 127,
     "cold": 166,
     "load": 2641,
     "long": 997,
     "long_xrt": 39.9
    },
    "directml": {
     "idle": 84,
     "warm": 46,
     "cold": 374,
     "load": 2559,
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
     "idle": 5245,
     "warm": 5178,
     "cold": 5894,
     "load": 1249,
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
     "idle": 760,
     "warm": 762,
     "cold": 803,
     "load": 2043,
     "long": 4861,
     "long_xrt": 8.2
    },
    "directml": {
     "idle": 1360,
     "warm": 1300,
     "cold": 1685,
     "load": 1991,
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
     "idle": 758,
     "warm": 791,
     "cold": 800,
     "load": 2004,
     "long": 5116,
     "long_xrt": 7.8
    },
    "directml": {
     "idle": 1382,
     "warm": 1266,
     "cold": 1602,
     "load": 1811,
     "long": 6884,
     "long_xrt": 5.8
    }
   }
  }
 ]
};
