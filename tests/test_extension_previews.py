"""Catalogue artwork through the same Rust queue used by the sheet host."""
from dataclasses import replace
import time
import zlib
import struct

import pytest

from ingetrazo_core import extension_manager as manager
from tests.extensions_chrome import Browser
from tests.test_extension_manager import _extension

REPO = "https://raw.githubusercontent.com/ingelibre/ingetrazo-extensions/main/"
MIRROR = "https://ingetrazo.com/extensiones-img/"
PRINT = "0123456789abcdef"


def png(rgb=(255, 0, 0)):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress((b"\x00" + bytes(rgb) * 2) * 2)) + chunk(b"IEND", b""))


def gallery(monkeypatch, root, count=1, **fields):
    entries = [replace(_extension(b"x", f"item{i}"), screenshot=f"item{i}.png", **fields) for i in range(count)]
    return Browser(monkeypatch, root, entries).act("tab", "Browse")


def visible(browser, start=0, stop=None):
    return browser.act("visible", *(it["icon"] for it in browser.items[start:stop]))


def complete(model, job, fetch, now=0):
    try:
        pixels = job.run(fetch)
    except (ValueError, OSError):
        model.preview_done(job.icon, now, None)
        raise
    model.preview_done(job.icon, now, pixels)
    return pixels


def test_switching_tabs_keeps_the_cards_and_their_screenshots(tmp_path, monkeypatch):
    browser = visible(gallery(monkeypatch, tmp_path, 3))
    model = browser.provider.model
    for job in model.jobs(0):
        complete(model, job, lambda *_: png())
    before = browser.items
    browser.act("tab", "Installed").act("tab", "Browse")
    assert browser.items == before and not model.jobs(1)
    assert set(browser.provider.build(browser.ctx).icons) == {it["icon"] for it in before}


def test_opening_decodes_only_the_visible_screenshots_and_a_same_refresh_keeps_the_cards(tmp_path, monkeypatch):
    browser = gallery(monkeypatch, tmp_path, 20)
    model = browser.provider.model
    assert not model.jobs(0) and not model.icons()
    visible(browser, 12, 16)
    jobs = model.jobs(0)
    assert [job.icon for job in jobs] == [it["icon"] for it in browser.items[12:15]]
    before = browser.items
    browser.act("verb", "refresh")
    if browser.provider.refreshing:
        browser.provider.refreshing.result(timeout=5)
    browser.read()
    assert browser.items == before and not model.jobs(1)


def test_a_screenshot_that_failed_is_asked_again_from_the_website(tmp_path, monkeypatch):
    browser = visible(gallery(monkeypatch, tmp_path, screenshot_sha=PRINT))
    model, urls = browser.provider.model, []
    for now, expected in [(0, REPO + "screenshots/item0.png"), (2000, MIRROR + "item0.png?v=" + PRINT),
                          (6000, REPO + "screenshots/item0.png"), (12000, MIRROR + "item0.png?v=" + PRINT)]:
        job, = model.jobs(now)
        with pytest.raises(ValueError):
            complete(model, job, lambda url, limit: urls.append(url) or b"corrupt", now)
        assert urls[-1] == expected and not model.jobs(now + 1999)
    assert not model.pending() and not model.jobs(24000)


def test_opening_the_manager_again_shows_every_card_with_its_screenshot(tmp_path, monkeypatch):
    browser = visible(gallery(monkeypatch, tmp_path, 6))
    model = browser.provider.model
    while jobs := model.jobs(0):
        for job in jobs:
            complete(model, job, lambda *_: png())
    browser.provider.closed(browser.ctx)
    again = gallery(monkeypatch, tmp_path, 6)
    assert set(again.provider.build(again.ctx).icons) == {it["icon"] for it in again.items}
    assert not visible(again).provider.model.jobs(1)


def test_the_background_rounds_fetch_the_missing_pictures_a_few_at_a_time(tmp_path, monkeypatch):
    from threading import Event
    browser = visible(gallery(monkeypatch, tmp_path, 12), 0, 3)
    p = browser.provider
    gate = Event()
    asked = []

    def fetch(url, limit):
        asked.append(url)
        assert gate.wait(5)
        return png()
    monkeypatch.setattr(manager, "download", fetch)
    try:
        p.signature(browser.ctx)
        assert len(p.requests) == 3
        gate.set()
        deadline = time.monotonic() + 5
        while p.model.pending() and time.monotonic() < deadline:
            p.signature(browser.ctx)
            assert len(p.requests) <= 3
            time.sleep(0.005)
        assert len(set(asked)) == 12 and len(p.model.icons()) == 12
    finally:
        gate.set()
        p.closed(browser.ctx)


def test_a_catalog_thumb_is_fetched_instead_of_the_screenshot(tmp_path, monkeypatch):
    browser = visible(gallery(monkeypatch, tmp_path, thumb="tiny.webp", screenshot_sha=PRINT))
    model, urls = browser.provider.model, []
    job, = model.jobs(0)
    complete(model, job, lambda url, limit: urls.append((url, limit)) or png())
    assert urls == [(REPO + "thumbs/tiny.webp", 8 << 20)]
    assert (browser.provider.cache / "catalog-thumbs" / f"tiny-{PRINT}.webp").read_bytes() == png()


def test_without_a_thumb_the_screenshot_is_fetched_under_its_print(tmp_path, monkeypatch):
    browser = visible(gallery(monkeypatch, tmp_path, screenshot_sha=PRINT))
    model, urls = browser.provider.model, []
    job, = model.jobs(0)
    complete(model, job, lambda url, limit: urls.append(url) or png())
    assert urls == [REPO + "screenshots/item0.png"]
    assert (browser.provider.cache / f"item0-{PRINT}.png").read_bytes() == png()


def test_a_new_print_under_the_same_name_is_fetched_again(tmp_path, monkeypatch):
    browser = visible(gallery(monkeypatch, tmp_path, screenshot_sha=PRINT))
    model = browser.provider.model
    old, = model.jobs(0)
    complete(model, old, lambda *_: png())
    cache = browser.provider.cache
    unrelated = cache / f"other-{PRINT}.png"
    unrelated.write_bytes(png())
    (cache / "item0.png").write_bytes(png())
    old_scaled, = (cache / "scaled").glob("*.png")
    updated = visible(gallery(monkeypatch, tmp_path, screenshot_sha="fedcba9876543210"))
    job, = updated.provider.model.jobs(0)
    assert job.icon != old.icon
    result = complete(updated.provider.model, job, lambda *_: png((0, 255, 0)))
    assert result[:4] == bytes((0, 255, 0, 255))
    assert unrelated.exists() and not (cache / "item0.png").exists()
    assert not (cache / f"item0-{PRINT}.png").exists() and not old_scaled.exists()


def test_a_thumb_github_failed_to_send_is_asked_from_the_website(tmp_path, monkeypatch):
    browser = visible(gallery(monkeypatch, tmp_path, thumb="tiny.webp", screenshot_sha=PRINT))
    model, urls = browser.provider.model, []
    job, = model.jobs(0)
    with pytest.raises(ValueError):
        complete(model, job, lambda url, _: urls.append(url) or b"corrupt")
    job, = model.jobs(2000)
    complete(model, job, lambda url, _: urls.append(url) or png())
    assert urls == [REPO + "thumbs/tiny.webp", MIRROR + "thumbs/tiny.webp"]
    assert not model.pending()
