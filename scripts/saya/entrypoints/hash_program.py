#!/usr/bin/env python3

import json

from starkware.cairo.bootloaders.hash_program import compute_program_hash_chain, HashFunction
from starkware.cairo.lang.compiler.program import Program


if __name__ == "__main__":
    compiled_program = Program.Schema().load(json.load(open("/program.json")))
    program_hash = hex(
        compute_program_hash_chain(program=compiled_program, program_hash_function=HashFunction.PEDERSEN)
    )

    print(f"Program hash:\n{program_hash}")
