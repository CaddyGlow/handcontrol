# HandControl Security Architecture

## Overview

HandControl uses mTLS (mutual TLS) for all communication after initial pairing. Two enrollment modes provide flexibility while maintaining security.

---

## Enrollment Mode Comparison

### QR Code Mode

```
┌─────────────┐                           ┌─────────────┐
│   Android   │                           │   PC Server │
└──────┬──────┘                           └──────┬──────┘
       │                                         │
       │  1. User generates QR on PC            │
       │◄────────────────────────────────────────┤
       │     QR contains:                        │
       │     - IP + Port                         │
       │     - Cert fingerprint (SHA256)         │
       │     - One-time enrollment token         │
       │                                         │
       │  2. Scan QR with camera                │
       │─────────────────────────────────────────┤
       │                                         │
       │  3. Connect via TLS                    │
       │════════════════════════════════════════►│
       │     Verify cert fingerprint matches QR  │
       │     (prevents MITM)                     │
       │                                         │
       │  4. Enroll(token, client_pubkey)       │
       │────────────────────────────────────────►│
       │                                         │
       │                     Server validates:   │
       │                     - Token valid?      │
       │                     - Token not expired?│
       │                     - Token not used?   │
       │                                         │
       │  5. EnrollResponse(success, client_id) │
       │◄────────────────────────────────────────┤
       │                                         │
       │  6. Pin server certificate (TOFU)      │
       │  7. Save client_id                     │
       │                                         │
       │  All future connections use mTLS       │
       │═══════════════════════════════════════►│
       │                                         │
```

**Security Properties:**
- ✅ MITM prevented by certificate fingerprint in QR
- ✅ One-time token prevents replay attacks
- ✅ Fast: immediate pairing after scan
- ⚠️ Requires physical access to PC display

---

### Approval Mode (with Verification Code)

```
┌─────────────┐                           ┌─────────────┐
│   Android   │                           │   PC Server │
└──────┬──────┘                           └──────┬──────┘
       │                                         │
       │  1. Discover via mDNS                  │
       │◄────────────────────────────────────────┤
       │     Service: _handcontrol._tcp.local.   │
       │     TXT: cert_fingerprint=SHA256:...    │
       │                                         │
       │  2. Generate client certificate        │
       │     (self-signed X.509, ECDSA P-256,    │
       │      10-year, hardware-backed)          │
       │     Store private key in Keystore       │
       │                                         │
       │  3. Connect via TLS (unverified)       │
       │════════════════════════════════════════►│
       │     ⚠️  Server cert not pinned yet      │
       │                                         │
       │  4. Compute verification code          │
       │     client_fp = SHA256(client_cert)    │
       │     server_fp = SHA256(server_cert)    │
       │     code = SHA256(client_fp ||         │
       │                   server_fp ||         │
       │                   server_id)           │
       │     Take first 6 digits: "482917"      │
       │     Format: "482-917"                  │
       │                                         │
       │  5. RequestPairing(full_cert, code)    │
       │────────────────────────────────────────►│
       │     Sends FULL certificate (DER)        │
       │                                         │
       │                     ⚠️ CRITICAL STEP:   │
       │                     Server validates:   │
       │                     1. Compute own code │
       │                     2. Compare codes    │
       │                     3. IF MISMATCH:     │
       │                        reject & return  │
       │                        error (MITM!)    │
       │                     4. IF MATCH:        │
       │                        show notification│
       │                                         │
       │                     Show notification:  │
       │                     "Device wants to    │
       │                      connect"           │
       │                     Code: 482-917       │
       │                     [Accept] [Reject]   │
       │                                         │
       │  6. Display verification code          │
       │     ┏━━━━━━━━━━━━━━━━━┓                │
       │     ┃   482-917       ┃                │
       │     ┗━━━━━━━━━━━━━━━━━┛                │
       │     "Verify code matches PC"           │
       │                                         │
       │  👤 USER VERIFIES CODES MATCH 👤       │
       │     If MITM: codes will differ!        │
       │                                         │
       │  7. Poll: CheckPairingStatus()         │
       │────────────────────────────────────────►│
       │     (every 2 seconds)                   │
       │                                         │
       │                     👤 User clicks      │
       │                     [Accept] after     │
       │                     verifying code     │
       │                                         │
       │  8. PairingStatus = APPROVED           │
       │◄────────────────────────────────────────┤
       │                                         │
       │  9. Pin server certificate (TOFU)      │
       │     Future connections verify cert!    │
       │  10. Save client_id                    │
       │                                         │
       │  All future connections use mTLS       │
       │═══════════════════════════════════════►│
       │                                         │
```

**Security Properties:**
- ✅ MITM prevented by verification code (out-of-band verification)
- ✅ User confirms correct server via visual code comparison
- ✅ No physical access to PC required (notification shown)
- ✅ Single-use pairing request ID prevents replay
- ⚠️ Requires user to actually check the codes match

---

## Why Verification Code Prevents MITM

### Scenario: Attacker tries MITM

```
┌─────────┐          ┌──────────┐          ┌──────────┐
│ Android │          │ Attacker │          │ PC Server│
└────┬────┘          └─────┬────┘          └────┬─────┘
     │                     │                    │
     │  RequestPairing()   │                    │
     ├────────────────────►│                    │
     │                     │  Forward request   │
     │                     ├───────────────────►│
     │                     │                    │
     │   Android computes: │   Attacker sees:  │   Server computes:
     │   code_A = SHA256(  │   Different certs!│   code_S = SHA256(
     │     client_pub,     │                    │     client_pub,
     │     ATTACKER_pub ❌ │                    │     server_pub ✅
     │   )                 │                    │   )
     │   = "482-917"       │                    │   = "719-283"  ≠≠≠
     │                     │                    │
     │   Shows: 482-917    │                    │   Shows: 719-283
     │   ┏━━━━━━━━━━━━┓    │                    │   Notification:
     │   ┃  482-917   ┃    │                    │   Code: 719-283
     │   ┗━━━━━━━━━━━━┛    │                    │
     │                     │                    │
     │  👤 USER SEES CODES DON'T MATCH! 👤      │
     │  User clicks [Reject] → Attack fails!    │
     │                     │                    │
```

**Key Insight:** Attacker cannot forge the verification code without the server's private key. The codes will differ if certificates are tampered with.

---

## Certificate Pinning (TOFU)

After successful first connection, Android pins the server's certificate:

```rust
// Pseudo-code
struct PinnedServer {
    server_id: Uuid,
    hostname: String,
    cert_fingerprint: [u8; 32],  // SHA256
    client_cert: Certificate,
}

// On connection:
fn connect(server: &PinnedServer) -> Result<Connection> {
    let conn = tls_connect(server.hostname)?;
    let actual_fingerprint = sha256(conn.peer_cert());

    if actual_fingerprint != server.cert_fingerprint {
        return Err("Certificate mismatch! Possible MITM attack");
    }

    // Certificate verified, proceed with mTLS
    conn.use_client_cert(server.client_cert);
    Ok(conn)
}
```

**Security Properties:**
- First connection uses verification code (TOFU - Trust On First Use)
- Future connections verify against pinned certificate
- Certificate changes detected immediately
- MITM impossible after initial pairing

---

## mTLS (Mutual TLS)

After enrollment, all connections use mutual TLS:

```
┌─────────────┐                           ┌─────────────┐
│   Android   │                           │   PC Server │
└──────┬──────┘                           └──────┬──────┘
       │                                         │
       │  1. TLS Handshake                      │
       │════════════════════════════════════════►│
       │     ClientHello                         │
       │                                         │
       │  2. Server sends certificate           │
       │◄════════════════════════════════════════┤
       │     ServerCertificate                   │
       │                                         │
       │  3. Client verifies:                   │
       │     - Cert matches pinned fingerprint  │
       │     - Cert not expired                 │
       │                                         │
       │  4. Client sends certificate           │
       │════════════════════════════════════════►│
       │     ClientCertificate                   │
       │                                         │
       │                     Server verifies:    │
       │                     - Cert in authorized│
       │                       clients list?     │
       │                     - Cert not revoked? │
       │                     - Cert valid?       │
       │                                         │
       │  5. ✅ Both authenticated               │
       │════════════════════════════════════════►│
       │     Encrypted channel established       │
       │                                         │
       │  6. ExecuteCommand(...)                │
       │────────────────────────────────────────►│
       │                                         │
```

**Security Properties:**
- ✅ Mutual authentication (both sides verify each other)
- ✅ Encrypted communication (TLS 1.3)
- ✅ Cannot connect without valid client certificate
- ✅ Server can revoke clients by removing cert from authorized list

---

## Threat Model

### Threats Mitigated

| Threat | Mitigation |
|--------|-----------|
| MITM at first connection (QR) | Certificate fingerprint in QR code |
| MITM at first connection (Approval) | Verification code (user verifies) |
| MITM after pairing | Certificate pinning + mTLS |
| Unauthorized device connection | mTLS client certificate required |
| Token replay (QR mode) | One-time enrollment tokens |
| Pairing request replay (Approval) | Single-use pairing request IDs |
| Command injection | Parameter sanitization + shell escaping |
| Eavesdropping | TLS 1.3 encryption |

### Threats NOT Mitigated (Acceptable)

| Threat | Why Not Mitigated | Acceptable? |
|--------|-------------------|-------------|
| Physical access to PC | Attacker can steal server private key | ✅ Yes - physical security assumed |
| Compromised Android device | Attacker can use valid client cert | ✅ Yes - same as user having malware |
| User ignores verification code | User clicks Accept without checking | ⚠️ User education required |
| Local network attacks | Attacker on same LAN | ✅ Yes - trusted network assumed |

---

## Best Practices for Users

### When Using QR Code Mode:
1. ✅ Only scan QR codes from your own PC
2. ✅ Don't share QR codes (they contain enrollment tokens)
3. ✅ Generate new QR for each device

### When Using Approval Mode:
1. ✅ **ALWAYS verify the codes match** before accepting
2. ✅ Reject if codes don't match (MITM attack!)
3. ✅ Only accept pairing requests you initiated
4. ✅ Check device name is correct

### General:
1. ✅ Revoke lost/stolen devices immediately
2. ✅ Keep server software updated
3. ✅ Run server as non-root user
4. ✅ Review authorized clients periodically

---

## Implementation Notes

### Certificate Algorithm Selection

**ECDSA P-256 (secp256r1) Benefits:**
- Smaller key sizes (256-bit vs 2048-bit RSA) = smaller certificates
- Better performance on mobile devices
- Hardware-backed support on modern Android devices (Keystore)
- Equivalent security to RSA-3072
- Widely supported by TLS implementations (rustls, OkHttp)

**Server Certificate:**
- Algorithm: ECDSA with P-256 curve
- Generated using `rcgen` crate with ECDSA key generation
- Self-signed for local network use

**Client Certificate (Android):**
- Algorithm: ECDSA with P-256 curve
- Hardware-backed generation via Android Keystore
- Private key never leaves secure hardware

### Verification Code Generation

```rust
use sha2::{Sha256, Digest};

fn generate_verification_code(
    client_cert_der: &[u8],  // Full DER-encoded ECDSA P-256 certificate
    server_cert_der: &[u8],  // Full DER-encoded ECDSA P-256 certificate
    server_id: &Uuid,
) -> String {
    // Compute fingerprints first
    let client_fingerprint = Sha256::digest(client_cert_der);
    let server_fingerprint = Sha256::digest(server_cert_der);

    // Combine fingerprints with server ID
    let mut hasher = Sha256::new();
    hasher.update(&client_fingerprint);
    hasher.update(&server_fingerprint);
    hasher.update(server_id.as_bytes());
    let hash = hasher.finalize();

    // Take first 6 digits from hex representation
    let hex = format!("{:x}", hash);
    let digits: String = hex.chars()
        .filter(|c| c.is_ascii_digit())
        .take(6)
        .collect();

    // Format as XXX-XXX
    format!("{}-{}", &digits[0..3], &digits[3..6])
}

// Server MUST validate before showing notification
fn handle_pairing_request(
    request: RequestPairingRequest,
    server_cert: &[u8],
    server_id: &Uuid,
) -> Result<PairingSession, PairingError> {
    // Compute expected verification code
    let expected_code = generate_verification_code(
        &request.client_certificate,
        server_cert,
        server_id,
    );

    // CRITICAL: Validate code BEFORE proceeding
    if request.verification_code != expected_code {
        return Err(PairingError::VerificationMismatch);
    }

    // Only if validation passes, create session and show notification
    let session = PairingSession::new(request.client_certificate);
    show_notification(&request.device_name, &expected_code);
    Ok(session)
}
```

### Certificate Pinning (Android)

```kotlin
// Store pinned certificate in Android Keystore
class CertificatePinner {
    fun pinServerCertificate(serverId: String, cert: X509Certificate) {
        val fingerprint = MessageDigest.getInstance("SHA-256")
            .digest(cert.encoded)

        // Store in secure Android Keystore
        secureStorage.put("server_cert_$serverId", fingerprint)
    }

    fun verifyCertificate(serverId: String, cert: X509Certificate): Boolean {
        val pinnedFingerprint = secureStorage.get("server_cert_$serverId")
            ?: return false

        val actualFingerprint = MessageDigest.getInstance("SHA-256")
            .digest(cert.encoded)

        return pinnedFingerprint.contentEquals(actualFingerprint)
    }
}
```

---

## Security Audit Checklist

- [ ] Verification code computation matches on both client and server
- [ ] Certificates are properly validated on both sides
- [ ] Client pins server certificate after first connection
- [ ] Server only accepts certificates in authorized list
- [ ] One-time tokens are invalidated after use
- [ ] Pairing request IDs expire after timeout
- [ ] Shell parameters are properly escaped
- [ ] Private keys never logged
- [ ] Verification codes never logged (only shown in UI/notification)
- [ ] TLS 1.3 with strong cipher suites
- [ ] Certificate expiry dates are reasonable (10 years)

---

**Last Updated:** 2025-10-29
