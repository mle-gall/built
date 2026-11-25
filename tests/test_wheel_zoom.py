"""Natural scrolling normalization is limited to Mac trackpads."""
import pytest
from ingetrazo_render import WheelEvent
from views import wheel_zoom


@pytest.mark.parametrize('platform', ['darwin', 'linux', 'win32'])
@pytest.mark.parametrize('touchpad', [False, True])
@pytest.mark.parametrize('natural', [False, True])
@pytest.mark.parametrize('delta', [-120, 120, 0, 30])
@pytest.mark.parametrize('override', [False, True])
def test_zoom_direction(monkeypatch, platform, touchpad, natural, delta, override):
    monkeypatch.setattr(wheel_zoom.sys, 'platform', platform)
    event = WheelEvent(100, 100, 0, delta if touchpad else 0, 0, delta,
                       "move", inverted=natural)
    expected = delta / 120
    if touchpad:
        expected *= wheel_zoom.TRACKPAD_ZOOM_GAIN
    if platform == 'darwin' and touchpad and natural:
        expected *= -1
    if override:
        expected *= -1
    assert wheel_zoom.zoom_steps(event, override) == expected
