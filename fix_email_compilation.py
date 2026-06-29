with open('/opt/swift/multidirectory-rust/src/handlers/email.rs') as f:
    c = f.read()

# Replace all SQL string continuations with concat approach
# Find SQL multi-line strings and fold them into single lines
import re

# Pattern: "SELECT ... \\\n         ... \\\n         ..."
# Replace with single-line strings (no \ continuations)
# Actually, let's just remove the \ continuation and merge lines
lines = c.split(chr(10))
new_lines = []
in_sql = False
sql_accum = []

for line in lines:
    stripped = line.strip()
    
    # Check if this line is part of a SQL string with \ continuation
    # Lines look like: '        "SELECT ... \'
    # or: '         FROM ... \'
    # The key: starts with a quoted string ending in \
    if (stripped.startswith('"SELECT') or stripped.startswith('"INSERT') or 
        stripped.startswith('"UPDATE') or stripped.startswith('"DELETE') or
        stripped.startswith('"SELECT id') or
        (in_sql and stripped.endswith(' \\')) or
        (in_sql and stripped.endswith('"'))) and not stripped.startswith('//'):
        in_sql = True
        # Remove the opening " or closing \"
        # We need to track the opening
        sql_accum.append(line)
        if stripped.endswith('"') and not stripped.endswith(' \\"') and in_sql:
            # End of SQL string - merge
            merged = ' '.join(
                s.strip().strip('"').rstrip(' \\').strip()
                for s in sql_accum
            )
            # First line determines indentation
            indent = ' ' * (len(sql_accum[0]) - len(sql_accum[0].lstrip()))
            # The opening quote
            first_line = sql_accum[0].lstrip()
            first_quote_end = first_line.index('"') + 1
            prefix = first_line[:first_quote_end]  # e.g. '"SELECT ...'
            
            new_lines.append(indent + '"' + merged + '"')
            in_sql = False
            sql_accum = []
        continue
    else:
        if in_sql:
            # If we were in a SQL string but this line doesn't look like continuation,
            # flush whatever we had
            merged = ' '.join(
                s.strip().strip('"').rstrip(' \\').strip()
                for s in sql_accum
            )
            indent = ' ' * (len(sql_accum[0]) - len(sql_accum[0].lstrip()))
            first_line = sql_accum[0].lstrip()
            first_quote_end = first_line.index('"') + 1
            prefix = first_line[:first_quote_end]
            
            new_lines.append(indent + '"' + merged + '"')
            in_sql = False
            sql_accum = []
        new_lines.append(line)

if in_sql and sql_accum:
    merged = ' '.join(
        s.strip().strip('"').rstrip(' \\').strip()
        for s in sql_accum
    )
    indent = ' ' * (len(sql_accum[0]) - len(sql_accum[0].lstrip()))
    first_line = sql_accum[0].lstrip()
    first_quote_end = first_line.index('"') + 1
    new_lines.append(indent + '"' + merged + '"')

c_new = chr(10).join(new_lines)
with open('/opt/swift/multidirectory-rust/src/handlers/email.rs', 'w') as f:
    f.write(c_new)
print('Done - merged SQL strings')