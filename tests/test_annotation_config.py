import dataclasses
import json
from pathlib import Path

import pytest

from epy_reader.models import Key
from epy_reader.settings import CfgDefaultKeymaps, Keymap, Settings


def test_annotation_key_conflict_is_rejected() -> None:
    from epy_reader.config import validate_annotation_keymaps

    keys = dataclasses.asdict(CfgDefaultKeymaps(AnnotationSelect="j"))
    mapping = Keymap(**{name: tuple(Key(char) for char in value) for name, value in keys.items()})
    with pytest.raises(ValueError, match="AnnotationSelect"):
        validate_annotation_keymaps(mapping)


@pytest.mark.parametrize("scrolloff", [-1, 2.5, "5", True, None])
def test_invalid_selection_scrolloff_is_rejected(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path, scrolloff: object
) -> None:
    from epy_reader.config import Config

    monkeypatch.setenv("HOME", str(tmp_path))
    configuration = tmp_path / ".config/epy/configuration.json"
    configuration.parent.mkdir(parents=True)
    settings = dataclasses.asdict(Settings())
    settings["AnnotationScrolloff"] = scrolloff
    configuration.write_text(
        json.dumps(dict(Setting=settings, Keymap=dataclasses.asdict(CfgDefaultKeymaps())))
    )
    with pytest.raises(ValueError, match="AnnotationScrolloff"):
        Config()
