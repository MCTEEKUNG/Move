# NetShare Internal Code Signing (LAN Only)

Use this workflow when you distribute `netshare-gui.exe` inside your own LAN/team and want Windows to trust your app.

## 1. Create internal code-signing certificate (build machine)

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\create-internal-code-signing-cert.ps1
```

Outputs:
- `certs\internal-code-signing\netshare-internal-code-signing.cer`
- `certs\internal-code-signing\netshare-internal-code-signing.pfx`

Keep `.pfx` private. Do not share it broadly.

## 2. Install trust certificate on target machine

Copy only `.cer` to the target machine and run:

```powershell
powershell -ExecutionPolicy Bypass -File .\install-internal-code-signing-cert.ps1 -CerPath .\netshare-internal-code-signing.cer
```

Use `-InstallForAllUsers` if you want machine-wide trust (requires admin).

## 3. Sign `netshare-gui.exe` on build machine

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\sign-netshare-gui.ps1 -ExePath .\target\release\netshare-gui.exe -PfxPath .\certs\internal-code-signing\netshare-internal-code-signing.pfx -PfxPassword "YOUR_PASSWORD"
```

## 4. Verify signature

```powershell
Get-AuthenticodeSignature .\target\release\netshare-gui.exe | Format-List Status, StatusMessage, SignerCertificate
```

## Notes

- This improves trust in your internal environment, but SmartScreen reputation still depends on distribution reputation.
- If files come from downloaded ZIPs, run `Unblock-File` before execution.
- Re-sign the executable every time you rebuild.
