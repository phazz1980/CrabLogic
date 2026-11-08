"""Variants of a sheet module where host calls go through one internal wrapper per import.

Usage: python tools/wrap_calls.py <in.wat> <out_prefix>
Writes <out_prefix>w.wat (direct call to the wrapper) and <out_prefix>wi.wat (call_indirect,
so LLVM cannot inline the wrapper back into every call site). Only `crab.text` and `crab.uart`
are wrapped: they are the calls with many arguments.
"""
import re, sys

src = open(sys.argv[1], encoding='utf-8').read()
imports = {}
for m in re.finditer(r'\(import "crab" "(\w+)" \(func \(;(\d+);\) \(type (\d+)\) ([^\n]*?)\)\)\n', src):
    imports[m.group(1)] = (int(m.group(2)), int(m.group(3)), m.group(4))
nfunc = len(re.findall(r'\(func \(;\d+;\)', src))
wrapped = [n for n in ("text", "uart") if n in imports]
wrappers, idx = [], {}
for k, name in enumerate(wrapped):
    fi, ti, sig = imports[name]
    nparams = len(re.findall(r'\bi32\b|\bf32\b', sig.split('(result')[0]))
    body = ''.join(f'    local.get {i}\n' for i in range(nparams))
    idx[name] = (fi, ti, nfunc + k)
    wrappers.append(f'  (func (;{nfunc + k};) (type {ti}) {sig}\n{body}    call {fi}\n  )\n')
end = src.index('  (data ')

def variant(indirect):
    s = src[:end]
    for name, (fi, ti, wi) in idx.items():
        k = wrapped.index(name)
        repl = f'i32.const {k}\n    call_indirect (type {ti})' if indirect else f'call {wi}'
        s = re.sub(rf'(\n\s*)call {fi}\n', lambda m: f'{m.group(1)}{repl}\n', s)
    s += ''.join(wrappers)
    if indirect:
        s += f'  (table (;0;) {len(wrapped)} {len(wrapped)} funcref)\n'
        s += '  (elem (;0;) (i32.const 0) func ' + ' '.join(str(idx[n][2]) for n in wrapped) + ')\n'
    return s + src[end:]

open(sys.argv[2] + 'w.wat', 'w', encoding='utf-8', newline='\n').write(variant(False))
open(sys.argv[2] + 'wi.wat', 'w', encoding='utf-8', newline='\n').write(variant(True))
