"""hex-convert: turn HuggingFace LLMs into hexrun-compatible bundles.

Two layers:

- ``manifest`` — the local, immediate-value layer. Take any Genie bundle
  produced by ``qai-hub-models`` (or pulled from Qualcomm's HuggingFace)
  and emit a ``hexrun.json`` manifest with sha256 sealing. No network,
  no AI Hub dependency. ARM64-friendly.
- ``export`` — the heavy layer. Wraps Qualcomm's ``qai-hub-models``
  export pipeline (HF -> ONNX -> AI-Hub cloud compile -> Genie bundle)
  and chains into ``manifest``. Requires an AI Hub API token, x64
  Python, network, and serious wall-clock time per model.

Plus an ``inspect`` command for pretty-printing what's in a bundle and
verifying sha256s.
"""

__version__ = "0.1.0.dev0"

from .export import ExportError, ExportOptions, export
from .inspect import inspect
from .manifest import ManifestOptions, write_manifest

__all__ = (
    "ManifestOptions",
    "write_manifest",
    "inspect",
    "ExportOptions",
    "ExportError",
    "export",
)
