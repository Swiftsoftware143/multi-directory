with open('/opt/swift/multidirectory-rust/src/handlers/email.rs') as f:
    c = f.read()

d = chr(36)

# Fix WHERE id = "" -> WHERE id =  (the  should be INSIDE the SQL string, not outside)
# The original had: WHERE id = "" which became WHERE id = "" incorrectly
# We need: WHERE id =  (with  inside the string)
c = c.replace('WHERE id = "' + d + '1"', 'WHERE id = ' + d + '1')
c = c.replace('WHERE id = ' + d + '1 \\', 'WHERE id = ' + d + '1 \\')
c = c.replace('DELETE FROM email_templates WHERE id = "' + d + '1"', 'DELETE FROM email_templates WHERE id = ' + d + '1')
c = c.replace('DELETE FROM email_campaigns WHERE id = "' + d + '1"', 'DELETE FROM email_campaigns WHERE id = ' + d + '1')

with open('/opt/swift/multidirectory-rust/src/handlers/email.rs', 'w') as f:
    f.write(c)

print('Fixed')