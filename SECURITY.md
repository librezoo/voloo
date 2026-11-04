# Security

Voloo switches power to real equipment, so please report problems privately first.

- Report vulnerabilities through GitHub's private vulnerability reporting
  (Security → Report a vulnerability) on this repository.
- Include the `voloo --version`, your OS, and the output of `voloo report` if it is relevant
  (it is a local file and contains no secrets, but read it before sharing).
- The local HTTP API listens on 127.0.0.1 only and requires a token; anything that bypasses
  that, or that makes a power command execute without an explicit request, is in scope.

This software is provided as is, without warranty (see [LICENSE](LICENSE)). Voloo only
switches externally supplied power; OFF does not mean the load terminals are at zero volts.
