import zipfile
import os

base_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), '../tests/fixtures')
os.makedirs(base_dir, exist_ok=True)

# 1. Simple archive, no comment
with zipfile.ZipFile(os.path.join(base_dir, 'simple_archive_no_comment.zip'), 'w') as z:
    z.writestr('test.txt', 'Hello World')

# 2. Archive with 65535b comment
with zipfile.ZipFile(os.path.join(base_dir, 'archive_with_65535b_comment.zip'), 'w') as z:
    z.writestr('test.txt', 'Hello World')
    z.comment = b'A' * 65535
