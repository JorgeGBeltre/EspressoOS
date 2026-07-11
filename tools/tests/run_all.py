#!/usr/bin/env python3
"""Runner unificado de los arneses de lógica pura de EspressoOS.

Descubre y ejecuta todos los `*_tests.py` de este directorio como suites de
`unittest`, y reporta un total agregado. Salida distinta de cero si algo falla
(apto para CI).

Ejecutar:  python tools/tests/run_all.py
"""
from __future__ import annotations

import glob
import os
import sys
import unittest


def main() -> int:
    here = os.path.dirname(os.path.abspath(__file__))
    sys.path.insert(0, here)

    modules = []
    for path in sorted(glob.glob(os.path.join(here, "*_tests.py"))):
        name = os.path.splitext(os.path.basename(path))[0]
        modules.append(name)

    if not modules:
        print("No se encontraron arneses *_tests.py", file=sys.stderr)
        return 1

    loader = unittest.TestLoader()
    suite = unittest.TestSuite()
    for name in modules:
        suite.addTests(loader.loadTestsFromName(name))

    print(f"Arneses: {', '.join(modules)}")
    result = unittest.TextTestRunner(verbosity=1).run(suite)

    total = result.testsRun
    failed = len(result.failures) + len(result.errors)
    print("-" * 60)
    print(f"TOTAL: {total} tests, {total - failed} OK, {failed} fallidos")
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
