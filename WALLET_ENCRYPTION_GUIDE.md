# Webcash Wallet Encryption Guide

This guide explains how wallet encryption works in both CLI and mobile scenarios, what happens behind the scenes, and how to manage encryption keys.

## **Overview**

Webcash wallets support two encryption modes:
1. **Password-based encryption** (CLI and fallback)
2. **Biometric encryption** (Mobile platforms)

The wallet database (SQLite file) can be encrypted at rest and decrypted for transactions.

---

## **CLI Usage Scenarios**

### **Scenario 1: Password-Based Encryption (CLI)**

#### Step 1: Create a Regular Wallet
```bash
webyc --wallet secure.db setup
```
- Creates a regular SQLite database
- Generates master secret using hardware RNG
- Wallet is **unencrypted** by default

#### Step 2: Encrypt the Database with Password
```bash
webyc --wallet secure.db encrypt-db --password
```
- Prompts: `Enter encryption password:`
- Encrypts the entire SQLite database with your password
- Database file becomes JSON (encrypted data)
- **Original database is overwritten with encrypted version**

#### Step 3: Use the Encrypted Wallet
To perform any operation on the encrypted wallet:

**Option A: Temporary decrypt for single operation**
```bash
# This won't work - database is encrypted
webyc --wallet secure.db info
❌ Failed to open wallet: Database error: file is not a database

# You need to decrypt first
webyc --wallet secure.db decrypt-db --password
# Prompts: Enter decryption password:
# Database is now decrypted back to SQLite

# Now you can use it normally
webyc --wallet secure.db info
webyc --wallet secure.db pay 0.001

# When done, encrypt it again
webyc --wallet secure.db encrypt-db --password
```

**Option B: Future enhancement - temporary runtime decryption**
```bash
# Future: This would decrypt to temp, run operation, encrypt back
webyc --wallet secure.db --password pay 0.001
# Behind the scenes:
# 1. Prompt for password
# 2. Decrypt database to /tmp/secure.temp.db
# 3. Perform payment operation
# 4. Encrypt back to secure.db
# 5. Delete /tmp/secure.temp.db
```

### **Behind the Scenes (CLI Password Mode)**

1. **Encryption**: 
   - Reads SQLite database bytes
   - Uses PBKDF2 + XOR encryption with your password
   - Writes JSON encrypted data back to same file

2. **Decryption**:
   - Reads JSON encrypted data
   - Uses your password to decrypt
   - Writes SQLite bytes back to same file

3. **Security**: Password is only in memory during operation

---

## **Mobile Usage Scenarios**

### **Scenario 2: Biometric Encryption (Mobile)**

#### Step 1: Create a Biometric Wallet
```bash
# CLI simulation of mobile wallet creation
webyc --wallet mobile.db setup --biometric
```
- Creates wallet with biometric encryption enabled
- Generates device-specific encryption key
- Stores encryption key in platform keychain/keystore

#### Step 2: App Runtime Behavior

**Opening the App:**
1. User opens wallet app
2. App prompts for Face ID/Touch ID
3. Upon successful biometric auth:
   - Retrieves encryption key from secure keychain
   - Decrypts wallet.db to temporary location
   - App uses decrypted database for operations

**Using the App:**
- All transactions work on decrypted temporary database
- User performs payments, checks balance, etc.
- Database remains decrypted in temp location during session

**Closing the App:**
1. App closes or goes to background
2. Encrypts temporary database with keychain-stored key
3. Overwrites main wallet.db with encrypted version
4. Deletes temporary decrypted database
5. Clears encryption key from memory

### **Behind the Scenes (Mobile Biometric Mode)**

**First Setup:**
```rust
// iOS Example
let encryption_key = CryptoSecret::generate(); // 32 random bytes
keychain.store(
    service: "com.webycash.wallet",
    account: "encryption_key", 
    data: encryption_key,
    access_control: kSecAccessControlBiometryAny
);
```

**Opening Wallet:**
```rust
// 1. Prompt for biometric authentication
let auth_success = biometric_prompt("Unlock Webcash Wallet").await;

// 2. Retrieve key from keychain (only works after biometric auth)
let encryption_key = keychain.retrieve(
    service: "com.webycash.wallet",
    account: "encryption_key"
)?;

// 3. Decrypt database to temp location
let temp_db = decrypt_database("wallet.db", &encryption_key)?;
let wallet = open_sqlite_database(temp_db);
```

**Closing Wallet:**
```rust
// 1. Read current temp database
let db_bytes = read_database(temp_db_path);

// 2. Encrypt with stored key
let encrypted_data = encrypt_with_key(db_bytes, &encryption_key);

// 3. Write back to main location
write_file("wallet.db", encrypted_data);

// 4. Clean up
remove_file(temp_db_path);
zeroize(encryption_key); // Clear from memory
```

---

## **Platform-Specific Key Management**

### **iOS Implementation**

```rust
// Store encryption key in iOS Keychain
let access_control = SecAccessControl::create_with_flags(
    kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
    kSecAccessControlBiometryAny, // Face ID or Touch ID
);

let keychain_item = [
    kSecClass: kSecClassGenericPassword,
    kSecAttrService: "com.webycash.wallet",
    kSecAttrAccount: "encryption_key",
    kSecValueData: encryption_key_data,
    kSecAttrAccessControl: access_control,
];

SecItemAdd(keychain_item);
```

**iCloud Sync**: Keys can be synced via iCloud Keychain if desired
**Security**: Keys never leave Secure Enclave on newer devices

### **Android Implementation**

```rust
// Store encryption key in Android Keystore
let key_generator = KeyGenerator::getInstance("AES", "AndroidKeyStore");

let key_spec = KeyGenParameterSpec::Builder::new(
    "webcash_wallet_key",
    KeyProperties::PURPOSE_ENCRYPT | KeyProperties::PURPOSE_DECRYPT
)
.setBlockModes(KeyProperties::BLOCK_MODE_GCM)
.setEncryptionPaddings(KeyProperties::ENCRYPTION_PADDING_NONE)
.setUserAuthenticationRequired(true)
.setUserAuthenticationParameters(
    0, // Require auth for every use
    KeyProperties::AUTH_BIOMETRIC_STRONG
)
.build();

key_generator.init(key_spec);
let secret_key = key_generator.generateKey();
```

**Security**: Keys stored in hardware security module when available

---

## **Key Management & Rotation**

### **Changing Encryption Key/Password**

#### CLI Password Change:
```bash
# 1. Decrypt with old password
webyc --wallet secure.db decrypt-db --password
# Enter old password

# 2. Encrypt with new password  
webyc --wallet secure.db encrypt-db --password
# Enter new password
```

#### Mobile Biometric Key Rotation:
```rust
// 1. Authenticate with current biometric
let current_key = keychain.retrieve_with_biometric_auth("encryption_key")?;

// 2. Decrypt wallet with current key
let wallet_data = decrypt_wallet(&current_key)?;

// 3. Generate new encryption key
let new_key = CryptoSecret::generate();

// 4. Store new key in keychain (overwrites old)
keychain.store_with_biometric_protection("encryption_key", &new_key)?;

// 5. Encrypt wallet with new key
encrypt_wallet(&wallet_data, &new_key)?;

// 6. Clear old key from memory
zeroize(current_key);
```

### **Emergency Recovery**

If biometric authentication fails or device is damaged:

1. **iOS**: Use device passcode as fallback
2. **Android**: Use device PIN/pattern as fallback  
3. **Export**: Export wallet to password-encrypted backup
4. **Import**: Import on new device and set up new biometric key

---

## **Security Considerations**

### **Threat Model Protection**

✅ **Device theft**: Wallet encrypted at rest
✅ **App backgrounding**: Database re-encrypted when app closes
✅ **Memory dumps**: Keys cleared from memory after use
✅ **Malware**: Keys require biometric authentication to access
✅ **Physical access**: Hardware-backed key storage

### **Attack Scenarios**

❌ **Biometric spoofing**: Depends on platform liveness detection
❌ **Keychain extraction**: Possible with device compromise + forensic tools
❌ **Side-channel attacks**: Theoretical against hardware modules

### **Best Practices**

1. **Regular backups**: Export encrypted wallet backups
2. **Key rotation**: Rotate encryption keys periodically
3. **Device security**: Keep OS updated, use strong device passcode
4. **App security**: Don't leave app open indefinitely

---

## **Future Enhancements**

### **Planned Features**

1. **Runtime password mode**: Temporary decrypt for single operations
2. **Hardware security keys**: YubiKey/FIDO2 support
3. **Multi-device sync**: Secure wallet sync across devices
4. **Key escrow**: Secure key backup and recovery
5. **Quantum resistance**: Post-quantum cryptography migration

### **CLI Enhancements**

```bash
# Future commands
webyc --wallet secure.db --password info    # Runtime decrypt
webyc --wallet secure.db rotate-key         # Key rotation
webyc --wallet secure.db backup --password  # Encrypted backup
webyc --wallet secure.db import backup.json # Restore from backup
```

---

This architecture provides strong security while maintaining usability across both CLI and mobile platforms. The key insight is that the wallet database itself gets encrypted/decrypted as needed, with platform-appropriate key management.