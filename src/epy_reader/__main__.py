import curses
import multiprocessing
import os
import sys

import epy_reader.cli as cli
import epy_reader.reader as reader


def main() -> None:
    # On Windows, calling this method is necessary
    # On Linux/OSX, this method does nothing
    multiprocessing.freeze_support()
    filepath, dump_only = cli.find_file()
    if dump_only:
        cli.dump_ebook_content(filepath)
        sys.exit()

    # NOTE: A short Escape timeout avoids the default one-second pause while preserving terminal key sequences.
    os.environ.setdefault("ESCDELAY", "25")
    while True:
        filepath = curses.wrapper(reader.start_reading, filepath)


# https://setuptools.pypa.io/en/latest/userguide/entry_point.html
if __name__ == "__main__":
    main()
