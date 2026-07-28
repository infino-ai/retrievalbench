#!/usr/bin/env python3
"""Remove VectorDBBench's run-mode UI from a source tree.

The hosted viewer is unauthenticated, so no control that starts a benchmark or
writes config may reach the image. Each edit is anchored to exact upstream
text and a missing anchor aborts the build, so upstream moving this code fails
loudly instead of restoring a public trigger.

Usage: strip_run_mode.py <path-to-vectordb_bench-package>
"""

import shutil
import sys
from pathlib import Path

# components/run_test holds widgets that only the removed pages import.
DELETED = (
    "pages/run_test.py",
    "pages/custom.py",
    "components/run_test",
)

NAV = "components/check_results/nav.py"
WELCOME = "components/welcome/welcomePrams.py"

NAV_BUTTON = '''def NavToRunTest(st):
    st.subheader("Run your test")
    st.write("You can set the configs and run your own test.")
    navClick = st.button("Run Your Test &nbsp;&nbsp;>")
    if navClick:
        st.switch_page("pages/run_test.py")
'''

NAV_BUTTON_STUB = '''def NavToRunTest(st):
    """No-op: the hosted viewer serves published results only."""
'''

NAV_RUN_TEST_LINK = '        {"name": "Run Test", "link": "run_test"},\n'
NAV_CUSTOM_LINK = '        {"name": "Custom Dataset", "link": "custom"},\n'

WELCOME_RUN_TEST_CARD = '''        {
            "title": "Run Test",
            "description": (
                "<span style='font-size: 17px;'>"
                "Select the databases and cases to test.<br>"
                "The test results will be displayed in Results."
                "</span>"
            ),
            "image": "fig/homepage/run_test.png",
            "link": "run_test",
        },
'''

WELCOME_CUSTOM_CARD = '''        {
            "title": "Custom Dataset",
            "description": (
                "<span style='font-size: 17px;'>"
                "Define users' own datasets with detailed descriptions of setting each parameter."
                "</span>"
            ),
            "image": "fig/homepage/custom.png",
            "link": "custom",
        },
'''

# The slice bound and the trailing row exist to place the two removed cards, so
# they go with them.
WELCOME_CARD_SLICE = "    for option in options[:8]:\n"
WELCOME_CARD_SLICE_ALL = "    for option in options:\n"

WELCOME_RUN_YOUR_OWN_ROW = '''    html_content += """
    </div>
    <div class="title-row">
        <h2>Run Your Own Test</h2>
    </div>
    <div class="last-row">
    """

    for option in options[8:10]:
        html_content += f"""
        <a href="/{option['link']}" target="_self" style="text-decoration: none;">
            <div class="section-card">
                <img src="{option['image']}" class="section-image" alt="{option['title']}">
                <div class="section-title">{option['title']}</div>
                <div class="section-description">{option['description']}</div>
            </div>
        </a>
        """

    html_content += """
    </div>
    """
'''

WELCOME_CLOSING_DIV = '''    html_content += """
    </div>
    """
'''


class AnchorMissing(RuntimeError):
    """Expected upstream source was not found."""


def remove(target: Path) -> None:
    """Delete a file or directory; raises if it is already absent."""
    if not target.exists():
        raise AnchorMissing(f"expected to remove {target}, but it does not exist")
    if target.is_dir():
        shutil.rmtree(target)
    else:
        target.unlink()


def substitute(source: Path, anchor: str, replacement: str) -> None:
    """Replace `anchor`; raises unless it matches exactly once."""
    text = source.read_text()
    matches = text.count(anchor)
    if matches != 1:
        raise AnchorMissing(f"{source}: expected exactly 1 match for anchor, found {matches}\n--- anchor ---\n{anchor}")
    source.write_text(text.replace(anchor, replacement))


def excise(source: Path, anchor: str) -> None:
    substitute(source, anchor, "")


def strip_run_mode(frontend: Path) -> None:
    """Delete the run-mode pages and every widget and link reaching them."""
    for relative in DELETED:
        remove(frontend / relative)

    nav = frontend / NAV
    substitute(nav, NAV_BUTTON, NAV_BUTTON_STUB)
    excise(nav, NAV_RUN_TEST_LINK)
    excise(nav, NAV_CUSTOM_LINK)

    welcome = frontend / WELCOME
    excise(welcome, WELCOME_RUN_TEST_CARD)
    excise(welcome, WELCOME_CUSTOM_CARD)
    substitute(welcome, WELCOME_CARD_SLICE, WELCOME_CARD_SLICE_ALL)
    substitute(welcome, WELCOME_RUN_YOUR_OWN_ROW, WELCOME_CLOSING_DIV)


def verify(frontend: Path) -> None:
    """Raise if any run-mode path or reference survived."""
    for relative in DELETED:
        if (frontend / relative).exists():
            raise AnchorMissing(f"{relative} still present after the strip")

    residue = []
    for source in sorted(frontend.rglob("*.py")):
        for number, line in enumerate(source.read_text().splitlines(), start=1):
            # A stale mention in a comment reaches no one; only live code counts.
            if line.lstrip().startswith("#"):
                continue
            if "run_test" in line or '"link": "custom"' in line:
                residue.append(f"{source.relative_to(frontend).as_posix()}:{number}: {line.strip()}")

    if residue:
        joined = "\n".join(residue)
        raise AnchorMissing(f"run-mode references survived the strip:\n{joined}")


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        sys.stderr.write(f"usage: {argv[0]} <path-to-vectordb_bench-package>\n")
        return 2

    frontend = Path(argv[1]).resolve() / "frontend"
    if not frontend.is_dir():
        sys.stderr.write(f"not a VectorDBBench package: {frontend} is missing\n")
        return 2

    try:
        strip_run_mode(frontend)
        verify(frontend)
    except AnchorMissing as error:
        sys.stderr.write(f"strip_run_mode: {error}\n")
        return 1

    sys.stdout.write(f"strip_run_mode: run-mode UI removed from {frontend}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
