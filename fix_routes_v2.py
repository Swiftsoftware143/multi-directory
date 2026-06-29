p = "/opt/swift/multidirectory-rust/src/routes.rs"
c = open(p).read()
lines = c.split("\n")
new_lines = []
skip_next = False
for i, line in enumerate(lines):
    if "/directories/:slug/phone-numbers" in line and "call_tracking::directory_phone_numbers" in line:
        # Remove this misplaced line entirely
        continue
    # Check if line 112 has the provision route
    if i > 0 and "phone-numbers/:id/provision" in lines[i-1] and ";" in lines[i-1] and not line.strip().startswith(".") and not line.strip().startswith("//"):
        # line after ; should be a blank or comment - already handled
        pass
    new_lines.append(line)
c_modified = "\n".join(new_lines)

# Now find the call_tracking section and add the route there
# Find line with "/phone-numbers/:id/provision"
insert_pos = c_modified.find(".route(\"/phone-numbers/:id/provision\"")
if insert_pos > 0:
    line_end = c_modified.find("\n", insert_pos)
    # Add the route BEFORE this line (so it becomes part of the chain before the ;)
    route_line = "\n        .route(\"/directories/:slug/phone-numbers\", get(call_tracking::directory_phone_numbers))"
    c_modified = c_modified[:line_end] + route_line + c_modified[line_end:]
open(p, "w").write(c_modified)
print("Fixed")