#!/usr/bin/env python3
"""Script to add max_tickets_per_address field and error to lib.rs"""

import re

# Read the file
with open('contracts/raffle-instance/src/lib.rs', 'r', encoding='utf-8') as f:
    content = f.read()

# 1. Add field to Raffle struct after max_tickets_per_tx
raffle_pattern = r'(pub max_tickets_per_tx: u32,)\n(\s+)(pub min_tickets: u32,)'
raffle_replacement = r'\1\n\2pub max_tickets_per_address: u32,\n\2\3'
content = re.sub(raffle_pattern, raffle_replacement, content)

# 2. Add error code after RandomnessTooEarly
error_pattern = r'(RandomnessTooEarly = 64,)\n(\})'
error_replacement = r'\1\n    ExceedsMaxTicketsPerAddress = 65,\n\2'
content = re.sub(error_pattern, error_replacement, content)

# Write back
with open('contracts/raffle-instance/src/lib.rs', 'w', encoding='utf-8') as f:
    f.write(content)

print("✓ Updated lib.rs: Added max_tickets_per_address field and ExceedsMaxTicketsPerAddress error")
