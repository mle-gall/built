"""Read the native title bar after AppKit has processed each window event."""
import inspect
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from scripts.isolated_launch import run

ROOT = Path(__file__).resolve().parents[1]
pytestmark = [pytest.mark.window, pytest.mark.skipif(
    sys.platform != "darwin" or os.getenv("BUILT_WINDOW_TESTS") != "1",
    reason="requires a macOS window session")]


def setup(app):
    import ctypes
    from pathlib import Path
    probe = ctypes.CDLL(str(Path(__file__).with_suffix(".dylib")))
    probe.sample.argtypes = [ctypes.c_char_p]
    for op in ("sample", "title", "edited", "light", "dark", "fullscreen"):
        app.add_menu_action(op, lambda op=op: probe.sample(op.encode()))


def test_native_window_buttons_follow_window_events(tmp_path):
    profile = tmp_path / "profile"
    plugin = profile / "data/built/plugins/button_frames.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text(inspect.getsource(setup))
    source = tmp_path / "probe.m"
    source.write_text(r"""
#import <AppKit/AppKit.h>
void sample(const char *operation) {
    NSMutableArray *windows = [NSMutableArray array];
    for (NSWindow *w in NSApp.windows) {
        if (!(w.styleMask & NSWindowStyleMaskFullSizeContentView) || ![w standardWindowButton:NSWindowCloseButton]) continue;
        if (!strcmp(operation,"fullscreen")) {
            w.collectionBehavior |= NSWindowCollectionBehaviorFullScreenPrimary;
            [w toggleFullScreen:nil];
        }
        if (!strcmp(operation,"title")) w.title = @"New document title";
        if (!strcmp(operation,"edited")) w.documentEdited = YES;
        if (!strcmp(operation,"light")) w.appearance = [NSAppearance appearanceNamed:NSAppearanceNameAqua];
        if (!strcmp(operation,"dark")) w.appearance = [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
        if (strcmp(operation,"sample")) continue;
        NSMutableArray *buttons = [NSMutableArray array];
        for (int k=0; k<3; ++k) {
            NSButton *b = [w standardWindowButton:k];
            NSRect r = [b convertRect:b.bounds toView:nil];
            NSRect content = [w.contentView convertRect:w.contentView.bounds toView:nil];
            [buttons addObject:@[@(NSMinX(r)), @(NSMaxY(content)-NSMidY(r)), @(NSWidth(r)), @(b.hidden)]];
        }
        [windows addObject:@{@"id":@(w.windowNumber), @"width":@(NSWidth(w.contentView.bounds)),
            @"fullscreen":@((w.styleMask & NSWindowStyleMaskFullScreen)!=0), @"buttons":buttons}];
    }
    if (!strcmp(operation,"sample")) {
        NSData *json = [NSJSONSerialization dataWithJSONObject:windows options:0 error:nil];
        FILE *f = fopen(getenv("BUILT_BUTTON_REPORT"), "a");
        fwrite(json.bytes,1,json.length,f); fputc('\n',f); fclose(f);
    }
}
""")
    subprocess.run(["clang", "-dynamiclib", "-framework", "AppKit", source,
                    "-o", plugin.with_suffix(".dylib")], check=True)
    sample = "wait 350\nmenu extension:button_frames:0"
    changes = ["", "resize 1600 900", "menu extension:button_frames:1",
               "menu extension:button_frames:2", "menu extension:button_frames:3",
               "menu extension:button_frames:4", "menu new",
               "menu extension:button_frames:5\nwait 1500", "menu extension:button_frames:5\nwait 1500",
               "menu new_window"]
    steps = tmp_path / "steps"
    steps.write_text("\n".join([f"{op}\n{sample}" for op in changes] + ["state", "quit"]))
    report = tmp_path / "frames.jsonl"
    binary = os.getenv("BUILT_BINARY", ROOT / "rust/built_app/target/release/built")
    result = run([binary, "--new-instance", "--root", ROOT, "--venv", ROOT / ".venv",
                  "--size", "1280x800", "--script", steps], settings=profile,
                 env=dict(os.environ, BUILT_BUTTON_REPORT=str(report)),
                 capture_output=True, text=True, timeout=90)
    assert result.returncode == 0 and report.exists(), result.stderr
    state = next(json.loads(line.removeprefix("[built] state ")) for line in result.stdout.splitlines()
                 if line.startswith("[built] state "))
    assert state["tab_rects"][0][0] == pytest.approx(90, abs=.5)
    samples = list(map(json.loads, report.read_text().splitlines()))
    assert len(samples) == len(changes)
    assert samples[0][0]["width"] == 1280
    assert samples[1][0]["width"] == 1600
    assert samples[-3][0]["fullscreen"]
    assert not samples[-2][0]["fullscreen"]
    assert len(samples[-1]) == 2
    for windows in samples:
        for window in windows:
            left = 16
            for x, y, width, hidden in window["buttons"]:
                assert hidden == window["fullscreen"]
                if not hidden:
                    assert (x, y) == pytest.approx((left, 20), abs=.5)
                left = x + width + 8
