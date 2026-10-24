"""The image-helper process contract and terminal capability checks."""

import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, List

import pytest
from image_helpers import build_png

from epy_reader.images.kitten import ImagePreviewError, clear_image, render_image
from epy_reader.images.models import GraphicsSupport, ImageRequest, ImageSource
from epy_reader.images.terminal import query_graphics_support


@dataclass
class HelperInvocation:
    arguments: List[str]
    image: bytes
    stdin: int
    detached: bool


class TestKittenRenderer:
    def test_helper_receives_a_noninteractive_render_request(
        self, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        source = ImageSource(name="cover.png", data=build_png())
        request = ImageRequest(
            rows=40, cols=120, cell_width_px=8, cell_height_px=21, image_id=42, in_tmux=True
        )
        captured = []

        def run(arguments: List[str], **options: Any) -> subprocess.CompletedProcess:
            captured.append(
                HelperInvocation(
                    arguments=arguments[:-1],
                    image=Path(arguments[-1]).read_bytes(),
                    stdin=options["stdin"],
                    detached=options["start_new_session"],
                )
            )
            return subprocess.CompletedProcess(
                arguments, 0, stdout=b"image escape codes", stderr=b""
            )

        monkeypatch.setattr(subprocess, "run", run)
        output = render_image("/bin/kitten", source, request)
        assert captured == [
            HelperInvocation(
                arguments=[
                    "/bin/kitten",
                    "icat",
                    "--stdin=no",
                    "--transfer-mode=stream",
                    "--unicode-placeholder",
                    "--loop=0",
                    "--scale-up",
                    "--use-window-size=120,40,960,840",
                    "--place=116x35@2x2",
                    "--image-id=42",
                    "--passthrough=tmux",
                ],
                image=source.data,
                stdin=subprocess.DEVNULL,
                detached=True,
            )
        ]
        assert output == "image escape codes"

    @pytest.mark.parametrize(
        "cols,rows,placement",
        [
            pytest.param(308, 64, "--place=296x59@6x2", id="wide"),
            pytest.param(120, 420, "--place=116x296@2x61", id="tall"),
            pytest.param(420, 420, "--place=296x296@62x61", id="both"),
        ],
    )
    def test_large_viewports_keep_images_inside_the_placeholder_limit(
        self, monkeypatch: pytest.MonkeyPatch, cols: int, rows: int, placement: str
    ) -> None:
        arguments = []

        def run(command: List[str], **options: Any) -> subprocess.CompletedProcess:
            arguments.extend(command)
            return subprocess.CompletedProcess(command, 0, stdout=b"image escape codes", stderr=b"")

        monkeypatch.setattr(subprocess, "run", run)
        render_image(
            "kitten",
            ImageSource(name="wide.png", data=build_png()),
            ImageRequest(rows=rows, cols=cols, image_id=42),
        )
        assert placement in arguments

    def test_helper_failures_explain_the_reason_without_terminal_escape_codes(
        self, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
            return subprocess.CompletedProcess(
                [],
                0,
                stdout=b"",
                stderr=b"\x1b[31mImage too large: maximum is 297x297 cells\x1b[39m\r\n",
            )

        monkeypatch.setattr(subprocess, "run", run)
        with pytest.raises(ImagePreviewError) as caught:
            render_image(
                "kitten",
                ImageSource(name="wide.png", data=build_png()),
                ImageRequest(rows=40, cols=120, image_id=42),
            )
        message = str(caught.value)
        assert "maximum is 297x297 cells" in message
        assert "\x1b" not in message
        assert "Press e" in message

    @pytest.mark.parametrize("returncode,output", [(1, b""), (0, b"")])
    def test_failed_image_processing_offers_external_viewing(
        self, monkeypatch: pytest.MonkeyPatch, returncode: int, output: bytes
    ) -> None:
        def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
            return subprocess.CompletedProcess([], returncode, stdout=output, stderr=b"failed")

        monkeypatch.setattr(subprocess, "run", run)
        with pytest.raises(ImagePreviewError, match="Cannot render"):
            render_image(
                "kitten",
                ImageSource(name="cover.png", data=build_png()),
                ImageRequest(rows=40, cols=120, image_id=42),
            )

    def test_stalled_image_processing_reports_a_preview_error(
        self, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
            raise subprocess.TimeoutExpired(cmd="kitten", timeout=10)

        monkeypatch.setattr(subprocess, "run", run)
        with pytest.raises(ImagePreviewError, match="Cannot render"):
            render_image(
                "kitten",
                ImageSource(name="cover.png", data=build_png()),
                ImageRequest(rows=40, cols=120, image_id=42),
            )

    def test_oversized_original_is_rejected_before_spawning_the_helper(self) -> None:
        with pytest.raises(ImagePreviewError, match="32 MiB"):
            render_image(
                "kitten",
                ImageSource(name="large.png", data=b"x" * (32 * 1024 * 1024 + 1)),
                ImageRequest(rows=40, cols=120, image_id=42),
            )

    def test_tiny_terminal_reports_unavailable_preview(self) -> None:
        with pytest.raises(ImagePreviewError, match="too small"):
            render_image(
                "kitten",
                ImageSource(name="cover.png", data=build_png()),
                ImageRequest(rows=4, cols=10, image_id=42),
            )

    @pytest.mark.parametrize("tmux", [False, True])
    def test_cleanup_targets_only_epys_image(self, tmux: bool) -> None:
        command = clear_image(42, in_tmux=tmux)
        if tmux:
            command = command[7:-2].replace("\x1b\x1b", "\x1b")
        assert command == "\x1b_Ga=d,d=I,i=42,q=2\x1b\\"


class TestTerminalSupport:
    @pytest.mark.parametrize("terminal", ["xterm-ghostty", "xterm-kitty"])
    def test_supported_direct_terminals_enable_preview(self, terminal: str) -> None:
        support = query_graphics_support(dict(TERM=terminal))
        assert support == GraphicsSupport()

    def test_unknown_terminal_explains_external_fallback(self) -> None:
        support = query_graphics_support(dict(TERM="xterm-256color"))
        assert "Ghostty or Kitty" in support.reason

    @pytest.mark.parametrize("passthrough", ["on", "all"])
    def test_tmux_client_controls_graphics_support(
        self, monkeypatch: pytest.MonkeyPatch, passthrough: str
    ) -> None:
        def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
            return subprocess.CompletedProcess(
                args=[], returncode=0, stdout="xterm-ghostty\t" + passthrough + "\t8\t21\n"
            )

        monkeypatch.setattr(subprocess, "run", run)
        support = query_graphics_support(dict(TMUX="test", TMUX_PANE="%4", TERM="tmux-256color"))
        assert support == GraphicsSupport(in_tmux=True, cell_width_px=8, cell_height_px=21)

    def test_disabled_tmux_passthrough_explains_the_required_setting(
        self, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
            return subprocess.CompletedProcess(
                args=[], returncode=0, stdout="xterm-ghostty\toff\t8\t21\n"
            )

        monkeypatch.setattr(subprocess, "run", run)
        support = query_graphics_support(dict(TMUX="test"))
        assert "allow-passthrough on" in support.reason

    def test_tmux_zero_pixel_metrics_use_readable_defaults(
        self, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
            return subprocess.CompletedProcess(
                args=[], returncode=0, stdout="xterm-ghostty\ton\t0\t0\n"
            )

        monkeypatch.setattr(subprocess, "run", run)
        assert query_graphics_support(dict(TMUX="test")) == GraphicsSupport(in_tmux=True)

    def test_failed_tmux_query_leaves_external_viewing_available(
        self, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
            raise subprocess.TimeoutExpired(cmd="tmux", timeout=1)

        monkeypatch.setattr(subprocess, "run", run)
        assert query_graphics_support(dict(TMUX="test")).reason is not None
