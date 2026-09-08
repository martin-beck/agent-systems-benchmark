# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Hostile-path tests for exact first-party source headers."""

import tempfile
import unittest
from pathlib import Path

from tools.quality.repository_policy import validate_sources

COPYRIGHT = "Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved."
SPDX = "SPDX-License-Identifier: MIT"


class SourceHeaderTests(unittest.TestCase):
    def check(self, name: str, content: str) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
            validate_sources([path], root=root)

    def reject(self, name: str, content: str) -> None:
        with self.assertRaisesRegex(
            ValueError,
            "exact adjacent Huawei 2026 and SPDX MIT",
        ):
            self.check(name, content)

    def test_valid_forms(self) -> None:
        self.check("source.rs", f"// {COPYRIGHT}\n// {SPDX}\nfn main() {{}}\n")
        self.check("source.py", f"# {COPYRIGHT}\n# {SPDX}\nprint('ok')\n")
        self.check("source.sh", f"#!/bin/sh\n# {COPYRIGHT}\n# {SPDX}\nexit 0\n")
        self.check("tools/awq", f"#!/bin/sh\n# {COPYRIGHT}\n# {SPDX}\nexit 0\n")
        self.check(
            "model.tla", f"---- MODULE Model ----\n\\* {COPYRIGHT}\n\\* {SPDX}\n====\n"
        )
        self.check(
            "model.als", f"module Model\n// {COPYRIGHT}\n// {SPDX}\nsig A {{}}\n"
        )

    def test_exact_extensionless_launcher_is_required(self) -> None:
        self.reject("tools/awq", "#!/bin/sh\nexit 0\n")

    def test_extensionless_lookalikes_are_not_selected(self) -> None:
        self.check("nested/tools/awq", "#!/bin/sh\nexit 0\n")
        self.check("tools/awq.exe", "#!/bin/sh\nexit 0\n")

    def test_formal_sources_require_headers_after_module_declaration(self) -> None:
        self.reject("model.tla", "---- MODULE Model ----\nEXTENDS Naturals\n")
        self.reject("model.als", "module Model\nsig A {}\n")

    def test_tla_header_cannot_displace_module_declaration(self) -> None:
        with self.assertRaisesRegex(
            ValueError,
            r"must begin with its TLA\+ module declaration",
        ):
            self.check(
                "model.tla",
                f"\\* {COPYRIGHT}\n\\* {SPDX}\n---- MODULE Model ----\n====\n",
            )

    def test_alloy_header_cannot_displace_module_declaration(self) -> None:
        with self.assertRaisesRegex(
            ValueError,
            "must begin with its Alloy module declaration",
        ):
            self.check(
                "model.als",
                f"// {COPYRIGHT}\n// {SPDX}\nmodule Model\nsig A {{}}\n",
            )

    def test_missing_copyright(self) -> None:
        self.reject("source.rs", f"// {SPDX}\n")

    def test_reversed(self) -> None:
        self.reject("source.py", f"# {SPDX}\n# {COPYRIGHT}\n")

    def test_intervening_line(self) -> None:
        self.reject("source.sh", f"#!/bin/sh\n# {COPYRIGHT}\n# comment\n# {SPDX}\n")

    def test_wrong_company(self) -> None:
        self.reject(
            "source.rs",
            f"// Copyright (C) Example Co. 2026. All rights reserved.\n// {SPDX}\n",
        )

    def test_wrong_year(self) -> None:
        self.reject(
            "source.rs",
            f"// Copyright (C) Huawei Technologies Co., Ltd. 2025. All rights reserved.\n"
            f"// {SPDX}\n",
        )

    def test_wrong_spacing(self) -> None:
        self.reject(
            "source.py",
            "#Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.\n"
            f"# {SPDX}\n",
        )

    def test_wrong_license(self) -> None:
        self.reject("source.py", f"# {COPYRIGHT}\n# SPDX-License-Identifier: Apache-2.0\n")

    def test_nonleading(self) -> None:
        self.reject("source.py", f"# comment\n# {COPYRIGHT}\n# {SPDX}\n")

    def test_duplicate_canonical_line(self) -> None:
        with self.assertRaisesRegex(ValueError, "exactly one canonical SPDX line"):
            self.check(
                "source.py",
                f"# {COPYRIGHT}\n# {SPDX}\n# {SPDX}\n",
            )


if __name__ == "__main__":
    unittest.main()
