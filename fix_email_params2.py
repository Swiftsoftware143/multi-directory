with open('/opt/swift/multidirectory-rust/src/handlers/email.rs') as f:
    c = f.read()

c = c.replace('VALUES (, , , , , ) \\', 'VALUES (, , , , , ) \\')
c = c.replace('WHERE id = "', 'WHERE id = ""')
c = c.replace('WHERE id =  \\', 'WHERE id =  \\')
c = c.replace('DELETE FROM email_templates WHERE id = "', 'DELETE FROM email_templates WHERE id = ""')
c = c.replace('DELETE FROM email_campaigns WHERE id = "', 'DELETE FROM email_campaigns WHERE id = ""')

with open('/opt/swift/multidirectory-rust/src/handlers/email.rs', 'w') as f:
    f.write(c)
print('done')