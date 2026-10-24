import dataclasses

import pytest

from epy_reader.models import Key
from epy_reader.settings import CfgDefaultKeymaps, Keymap


def test_annotation_key_conflict_is_rejected() -> None:
    from epy_reader.config import validate_annotation_keymaps

    keys = dataclasses.asdict(CfgDefaultKeymaps(AnnotationSelect="j"))
    mapping = Keymap(**{name: tuple(Key(char) for char in value) for name, value in keys.items()})
    with pytest.raises(ValueError, match="AnnotationSelect"):
        validate_annotation_keymaps(mapping)
