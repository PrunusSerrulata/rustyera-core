"""Check macOS runtime selection without starting Wine or building an oracle."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().with_name("test-macos-wine.sh")


class WineEnvironmentTests(unittest.TestCase):
    def test_selected_runtime_and_nls_reach_winepath(self):
        for use_default, nls in ((True, None), (False, "0")):
            with self.subTest(default=use_default, nls=nls), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                default_bin = root / "Library/Application Support/com.franke.Whisky/Libraries/Wine/bin"
                default_bin.mkdir(parents=True)
                binaries = default_bin if use_default else root / "override runtime/bin"
                binaries.mkdir(parents=True, exist_ok=True)
                if not use_default:
                    for name in ("wine", "winepath", "wineboot", "wineserver"):
                        tool = default_bin / name
                        tool.write_text("#!/bin/bash\necho wrong-runtime >&2\nexit 74\n")
                        tool.chmod(0o755)
                prefix = root / "prefix"
                prefix.mkdir()
                (prefix / "system.reg").touch()
                for name in ("wine", "winepath", "wineboot", "wineserver"):
                    tool = binaries / name
                    tool.write_text(
                        '#!/bin/bash\n'
                        'echo "selected:$WINEPREFIX:$DOTNET_SYSTEM_GLOBALIZATION_USENLS" >&2\n'
                        'exit 73\n'
                    )
                    tool.chmod(0o755)
                env = os.environ.copy()
                env.pop("EMUERA_WINE_BIN", None)
                env.pop("DOTNET_SYSTEM_GLOBALIZATION_USENLS", None)
                env.update(HOME=str(root), EMUERA_REFERENCE_WINE_PREFIX=str(prefix),
                           EMUERA_REFERENCE_WORK_DIR=str(root / "work"))
                if not use_default:
                    env["EMUERA_WINE_BIN"] = str(binaries)
                if nls is not None:
                    env["DOTNET_SYSTEM_GLOBALIZATION_USENLS"] = nls
                result = subprocess.run(["/bin/bash", str(SCRIPT)], env=env,
                                        capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 73, result.stderr)
                self.assertIn(f"selected:{prefix}:{nls or '1'}", result.stderr)
                self.assertNotIn("wrong-runtime", result.stderr)
                self.assertFalse(list((root / "work").glob("fixture.*")))

    def test_partial_runtime_fails_before_creating_work_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            wine = root / "wine"
            wine.write_text("#!/bin/bash\nexit 0\n")
            wine.chmod(0o755)
            env = dict(os.environ, EMUERA_WINE_BIN=str(root),
                       EMUERA_REFERENCE_WORK_DIR=str(root / "work"))
            result = subprocess.run(["/bin/bash", str(SCRIPT)], env=env,
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 127)
            self.assertIn(f"{root}/winepath", result.stderr)
            self.assertFalse((root / "work").exists())


if __name__ == "__main__":
    unittest.main()
