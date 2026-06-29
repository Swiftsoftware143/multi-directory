# Simple script to fix the  bind parameters in email.rs
with open('/opt/swift/multidirectory-rust/src/handlers/email.rs') as f:
    c = f.read()

# The file has VALUES (,,,) but needs VALUES (,,...)
# Let's just see what's there
import re

# Find all VALUES lines
for i, line in enumerate(c.split(chr(10))):
    if 'VALUES' in line:
        print('Found VALUES on line', i+1, ':', repr(line))
    if 'WHERE id =' in line:
        print('Found WHERE on line', i+1, ':', repr(line))
    if 'DELETE FROM' in line:
        print('Found DELETE on line', i+1, ':', repr(line))

print('Total  count:', c.count(''))
print('Total  count:', c.count(''))
print('Total  count:', c.count(''))
print('Total  count:', c.count(''))
print('Total  count:', c.count(''))
print('Total  count:', c.count(''))
print('Total  count:', c.count(''))