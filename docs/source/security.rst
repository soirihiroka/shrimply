Security
========

Blender and Manim files can execute arbitrary code with your account's
permissions. Shrimply asks for approval before loading untrusted executable
sources. Choose **Cancel**, **Trust N files**, or **Trust N folders**.

File trust covers future edits and replacements at that path. Folder trust
includes all current and future files and subfolders. Approvals are stored in
local settings, outside the project, and can be removed in
**Preferences**. Moving sources outside trusted locations requires
approval again.

Trust also authorizes code the source imports or invokes. It is not a sandbox
or malware scan; only approve sources you control or whose authors you trust.
