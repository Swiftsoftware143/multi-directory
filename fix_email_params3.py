with open('/opt/swift/multidirectory-rust/src/handlers/email.rs') as f:
    c = f.read()

d = chr(36)

c = c.replace('VALUES (, , , , , ) \\', 'VALUES (' + d + '1, ' + d + '2, ' + d + '3, ' + d + '4, ' + d + '5, ' + d + '6) \\')
c = c.replace('WHERE id = ""', 'WHERE id = "' + d + '1"')
c = c.replace('WHERE id =  \\', 'WHERE id = ' + d + '1 \\')
c = c.replace('DELETE FROM email_templates WHERE id = ""', 'DELETE FROM email_templates WHERE id = "' + d + '1"')
c = c.replace('DELETE FROM email_campaigns WHERE id = ""', 'DELETE FROM email_campaigns WHERE id = "' + d + '1"')

with open('/opt/swift/multidirectory-rust/src/handlers/email.rs', 'w') as f:
    f.write(c)
print('Done - replaced with chr(36) params')