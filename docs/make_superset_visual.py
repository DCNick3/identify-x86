from pwn import *
import click

# from llvm-tblgen.exe sha1 3e64f9d92a10fe930f0cec7ed5cc6af4ba6e14e0
#example_vma = 0x004415D4
#example_raw = b"\x55\x8B\xEC\x8B\x45\x08\x8B\x48\x1C\x85\xC9\x75\x10\xFF\x75\x10\x8B\x48\x18\xFF\x75\x0C\xE8\x6F\x26\xFD\xFF\x5D\xC3\x80\x79\x04\x05\x75\x16\xFF\x75\x10\x8B\x49\x0C\xFF\x75\x0C\xE8\x59\x26\xFD\xFF\x84\xC0\x74\x04\xB0\x01\x5D\xC3"

# example_vma = 0x100
# example_raw = b"\x8D\x43\x04\xC6\x00\x2A\x83\xEB\x04\x0F\x85\xEE\xFF\xFF\xFF"

example_vma = 0x401000
example_raw = b"\x8B\x4C\x24\x04\x8B\x01\x85\xC0\x7D\x02\xF7\xD8\x89\x01\xC3"

def parse_disasm_line(line):
    addr, rest = line.split(':', maxsplit=1)
    addr = int(addr, 16)
    raw = rest[:31].strip()
    mnemonic = rest[31:].strip()
    return addr, raw, mnemonic

def extract_addr(line):
    return parse_disasm_line(line)[0]

true_addrs = set(map(extract_addr, disasm(example_raw, vma=example_vma).split('\n')))

def colorify_disasm_line(line):
    addr, raw, mnemonic = parse_disasm_line(line)

    instr_color = 'green' if addr in true_addrs else 'red'
    return f"{click.style('%08x' % (addr), fg='yellow')}: {click.style(raw, fg='blue'):31} {click.style(mnemonic, fg=instr_color)}"


for i in range(0, len(example_raw)):
    d = disasm(example_raw[i:], vma=example_vma+i).split('\n')[0]
    print(colorify_disasm_line(d))
