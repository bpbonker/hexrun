# installer/

MSIX packaging assets for `hexrun`.

- `AppxManifest.xml` — package manifest. Wraps `hexrun.exe` as a Win32
  desktop application via `Windows.FullTrustApplication` +
  `runFullTrust`. Update the `Version` line on each release.
- `Assets/` — generated on first run of `scripts\build-msix.ps1` if
  not present. Solid-color placeholder PNGs at the sizes the
  manifest references. **Replace these with real artwork before
  shipping a public build.**

To build:

```powershell
pwsh -File scripts\build-msix.ps1
```

To sign (Phase 6, when we have a real cert):

```powershell
pwsh -File scripts\build-msix.ps1 -CertThumbprint <sha1>
```

The unsigned `.msix` is only installable on machines with Windows
developer mode enabled (`Settings > For developers`). Once signed and
the user trusts the publisher, the MSIX installs by double-click.

A portable `.zip` build (no developer-mode requirement) is produced by
`scripts\package-release.ps1`.
