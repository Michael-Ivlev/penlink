# Third-party notices

penlink-host.exe embeds and installs the following unmodified binaries (`host/driver/sudovda/`),
taken from the Apollo 0.4.6 release.

## SudoVDA (SudoVDA.dll, SudoVDA.inf, sudovda.cat, sudovda.cer)

Virtual display driver by SudoMaker. https://github.com/SudoMaker/SudoVDA
License: MIT. Copyright (c) SudoMaker. The full MIT text is in the SudoVDA repository (`LICENSE`).

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions: the above copyright notice and this
permission notice shall be included in all copies or substantial portions of the Software.
THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.

The driver is signed with a self-signed certificate (`sudovda.cer`, CN=sudovda@su.mk). Installing
the driver adds that certificate to the Windows "Trusted Root Certification Authorities" and
"Trusted Publishers" stores of this PC; `penlink-host.exe --uninstall-driver` removes it again.

## nefconc.exe (nefcon)

Driver installation helper by Nefarius Software Solutions. https://github.com/nefarius/nefcon
License: BSD-3-Clause. Copyright (c) Nefarius Software Solutions e.U.

Redistribution and use in source and binary forms, with or without modification, are permitted
provided that the following conditions are met: (1) redistributions of source code must retain the
above copyright notice, this list of conditions and the following disclaimer; (2) redistributions
in binary form must reproduce the above copyright notice, this list of conditions and the following
disclaimer in the documentation and/or other materials provided with the distribution; (3) neither
the name of the copyright holder nor the names of its contributors may be used to endorse or promote
products derived from this software without specific prior written permission.
THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
IMPLIED WARRANTIES ARE DISCLAIMED. See the nefcon repository (`LICENSE`) for the full text.
