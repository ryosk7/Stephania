"""SPDX licence-expression policy of ci/sbom.py (docs/LICENSING.md)."""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("sbom", pathlib.Path(__file__).resolve().parent / "sbom.py")
sbom = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sbom)


class SpdxPolicy(unittest.TestCase):
    def test_permissive(self):
        for expr in ("MIT", "MIT OR Apache-2.0", "(MIT OR Apache-2.0) AND Unicode-3.0", "MIT/Apache-2.0",
                     "Apache-2.0 WITH LLVM-exception", "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
                     "GPL-3.0 OR (MIT AND ISC)", "(MIT)", "MIT or Apache-2.0", "((MIT OR GPL-2.0) AND (ISC OR LGPL-2.1))"):
            self.assertTrue(sbom.spdx_ok(expr), expr)

    def test_conjunction_with_copyleft_is_refused(self):
        # Stripping parentheses used to turn `... OR MIT` into a standalone
        # alternative, so these passed although LGPL-2.1/GPL-3.0 is required.
        for expr in ("(MIT OR GPL-3.0) AND LGPL-2.1", "LGPL-2.1 AND (GPL-3.0 OR MIT)", "(MIT AND GPL-3.0)",
                     "MIT AND GPL-3.0", "(MIT OR Apache-2.0) AND (GPL-2.0 OR LGPL-3.0)", "GPL-3.0", "GPL-2.0+",
                     "Apache-2.0 WITH Classpath-exception-2.0"):
            self.assertFalse(sbom.spdx_ok(expr), expr)

    def test_and_binds_tighter_than_or(self):
        self.assertTrue(sbom.spdx_ok("GPL-3.0 AND LGPL-2.1 OR MIT"))
        self.assertFalse(sbom.spdx_ok("GPL-3.0 AND (LGPL-2.1 OR MIT)"))
        self.assertTrue(sbom.spdx_ok("MIT OR GPL-3.0 AND LGPL-2.1"))

    def test_malformed_or_missing_is_refused(self):
        for expr in (None, "", "  ", "MIT AND", "OR MIT", "(MIT", "MIT)", "MIT OR (", "MIT WITH", "MIT ISC", "()"):
            self.assertFalse(sbom.spdx_ok(expr), repr(expr))


if __name__ == "__main__":
    unittest.main()
