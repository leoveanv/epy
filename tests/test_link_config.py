import dataclasses

import pytest

from epy_reader.models import Key
from epy_reader.settings import CfgDefaultKeymaps, Keymap


def test_forward_binding_cannot_steal_table_of_contents() -> None:
    from epy_reader.config import validate_navigation_keymaps

    defaults = dataclasses.asdict(CfgDefaultKeymaps(JumpForward="t"))
    keymap = Keymap(
        **{name: tuple(Key(char) for char in value) for name, value in defaults.items()}
    )
    with pytest.raises(ValueError, match="JumpForward conflicts with TableOfContents"):
        validate_navigation_keymaps(keymap)
