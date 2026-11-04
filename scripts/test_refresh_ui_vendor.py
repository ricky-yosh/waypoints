import importlib.util
from pathlib import Path
import tomllib
import unittest


spec = importlib.util.spec_from_file_location(
    "refresh_ui_vendor", Path(__file__).with_name("refresh-ui-vendor.py")
)
vendor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vendor)


class VendorSourceTests(unittest.TestCase):
    def test_hot_reload_reaches_the_selected_engine(self):
        data = tomllib.loads(vendor.patch_manifest("gpui-kit", "[features]\n"))
        self.assertEqual(
            data["features"]["hot-reload"],
            ["gpui-fast?/hot-reload"],
        )

    def test_fast_accessibility_versions_are_aligned(self):
        for name in ["gpui-fast", "gpui-fast-macos", "gpui-fast-linux", "gpui-fast-windows"]:
            with self.subTest(name=name):
                manifest = """[dependencies.accesskit]
version = "0.24.0"

[target.'cfg(target_os = "macos")'.dependencies.accesskit_macos]
version = "0.26.0"

[dependencies.accesskit_unix]
version = "0.22.0"

[dependencies.accesskit_windows]
version = "0.33.0"
"""
                data = tomllib.loads(vendor.patch_manifest(name, manifest))
                self.assertEqual(data["dependencies"]["accesskit"]["version"], "0.25.2")
                self.assertEqual(
                    data["target"]['cfg(target_os = "macos")']["dependencies"]
                    ["accesskit_macos"]["version"],
                    "0.27.2",
                )
                self.assertEqual(data["dependencies"]["accesskit_unix"]["version"], "0.24.1")
                self.assertEqual(data["dependencies"]["accesskit_windows"]["version"], "0.35.2")

    def test_demo_sources_and_assets_are_omitted(self):
        for path in [
            "examples/gif_viewer.rs",
            "examples/image/black-cat-typing.gif",
            "examples/image/image.rs",
        ]:
            with self.subTest(path=path):
                self.assertFalse(vendor.keep_source("gpui-fast", Path(path)))

    def test_test_fixture_and_library_sources_are_preserved(self):
        for path in [
            "examples/image/exif-orientation-rotate-180.jpg",
            "src/platform.rs",
            "tests/action_macros.rs",
            "Cargo.toml",
            "README.md",
            "LICENSE-APACHE",
        ]:
            with self.subTest(path=path):
                self.assertTrue(vendor.keep_source("gpui-fast", Path(path)))

    def test_example_targets_are_removed_without_changing_other_targets(self):
        manifest = """[package]
name = "gpui-fast"
version = "0.1.13"
autoexamples = false

[[example]]
name = "gif_viewer"
path = "examples/gif_viewer.rs"

[[example]]
name = "image"
path = "examples/image/image.rs"
required-features = ["test-support"]

[[test]]
name = "action_macros"
path = "tests/action_macros.rs"

[dependencies.accesskit]
version = "0.24.0"
"""
        data = tomllib.loads(vendor.patch_manifest("gpui-fast", manifest))
        self.assertNotIn("example", data)
        self.assertFalse(data["package"]["autoexamples"])
        self.assertEqual(data["test"][0]["path"], "tests/action_macros.rs")
        self.assertEqual(data["dependencies"]["accesskit"]["version"], "0.25.2")


if __name__ == "__main__":
    unittest.main()
