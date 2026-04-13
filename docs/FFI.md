# FFI Reference

Complete C API reference for `webylib`. All functions are exported with C linkage and can be called from any language that supports C FFI.

## Building

```bash
# Shared library (.so / .dylib / .dll) + static library (.a / .lib)
cargo build --release --features ffi

# Generate C header
cbindgen --crate webylib --output include/webylib.h
```

Output files in `target/release/`:
- Linux: `libwebylib.so`, `libwebylib.a`
- macOS: `libwebylib.dylib`, `libwebylib.a`
- Windows: `webylib.dll`, `webylib.lib`
- iOS: `libwebylib.a` (static only)
- Android: `libwebylib.so` (shared only)

## Conventions

- Every function returns `int32_t` (`0` = success, non-zero = error code)
- On failure, call `weby_last_error_message()` for a human-readable description
- Strings returned via `out_*` pointers must be freed with `weby_free_string()`
- Wallet handles must be freed with `weby_wallet_free()`
- The `weby_version()` pointer is static — do **not** free it
- The `weby_last_error_message()` pointer is thread-local, valid until next FFI call — do **not** free it

## Error Codes

```c
enum WebyErrorCode {
    WEBY_OK               =  0,
    WEBY_INVALID_INPUT     =  1,
    WEBY_DATABASE_ERROR    =  2,
    WEBY_CRYPTO_ERROR      =  3,
    WEBY_SERVER_ERROR      =  4,
    WEBY_INSUFFICIENT_FUNDS =  5,
    WEBY_NETWORK_ERROR     =  6,
    WEBY_AUTH_ERROR         =  7,
    WEBY_NOT_SUPPORTED     =  8,
    WEBY_UNKNOWN           = -1,
};
```

---

## Functions

### Lifecycle

#### `weby_wallet_open`

Open or create a wallet at the given filesystem path.

```c
int32_t weby_wallet_open(
    const char *path,         // null-terminated UTF-8 path
    WebyWallet **out_wallet   // receives wallet handle on success
);
```

#### `weby_wallet_open_with_seed`

Open or create a wallet with a caller-provided 32-byte seed. If the wallet already has a different master secret with existing transactions, returns `WEBY_INVALID_INPUT`.

```c
int32_t weby_wallet_open_with_seed(
    const char *path,         // null-terminated UTF-8 path
    const uint8_t *seed_ptr,  // pointer to 32-byte seed
    size_t seed_len,          // must be 32
    WebyWallet **out_wallet   // receives wallet handle on success
);
```

#### `weby_wallet_free`

Free a wallet handle. Safe to call with NULL (no-op).

```c
void weby_wallet_free(WebyWallet *wallet);
```

### Operations

#### `weby_wallet_balance`

Get the wallet balance as a decimal string (e.g., `"1.50000000"`).

```c
int32_t weby_wallet_balance(
    const WebyWallet *wallet,
    char **out_balance        // receives allocated string — free with weby_free_string()
);
```

#### `weby_wallet_insert`

Insert webcash into the wallet. Performs ownership transfer via the server (replace operation).

```c
int32_t weby_wallet_insert(
    const WebyWallet *wallet,
    const char *webcash_str   // e.g., "e1.00000000:secret:abcdef..."
);
```

#### `weby_wallet_pay`

Pay an amount from the wallet. Returns the payment webcash string for the recipient.

```c
int32_t weby_wallet_pay(
    const WebyWallet *wallet,
    const char *amount_str,   // decimal amount, e.g., "0.5"
    const char *memo,         // memo string (or NULL)
    char **out_webcash        // receives payment string — free with weby_free_string()
);
```

#### `weby_wallet_check`

Verify all wallet outputs against the server (detect spent outputs).

```c
int32_t weby_wallet_check(const WebyWallet *wallet);
```

#### `weby_wallet_merge`

Consolidate small outputs into fewer larger ones.

```c
int32_t weby_wallet_merge(
    const WebyWallet *wallet,
    uint32_t max_outputs,     // max outputs to merge per batch
    char **out_result         // receives summary string — free with weby_free_string()
);
```

#### `weby_wallet_recover`

Recover wallet contents from a master secret by scanning the server.

```c
int32_t weby_wallet_recover(
    const WebyWallet *wallet,
    const char *master_secret_hex,  // 64-character hex string
    uint32_t gap_limit,             // scan depth per chain (typically 20)
    char **out_result               // receives summary — free with weby_free_string()
);
```

### Inspection

#### `weby_wallet_stats`

Get wallet statistics as a JSON string.

```c
int32_t weby_wallet_stats(
    const WebyWallet *wallet,
    char **out_json           // receives JSON — free with weby_free_string()
);
```

Response format:
```json
{
  "total_webcash": 10,
  "unspent_webcash": 8,
  "spent_webcash": 5,
  "total_balance": "3.50000000"
}
```

#### `weby_wallet_export_snapshot`

Export full wallet state as a JSON string (for backup).

```c
int32_t weby_wallet_export_snapshot(
    const WebyWallet *wallet,
    char **out_json           // receives JSON — free with weby_free_string()
);
```

### Encryption

#### `weby_wallet_encrypt_seed`

Encrypt the wallet database with a password (Argon2 + AES-256-GCM).

```c
int32_t weby_wallet_encrypt_seed(
    const WebyWallet *wallet,
    const char *password      // null-terminated password
);
```

### Utilities

#### `weby_version`

Get the library version string. The returned pointer is **static** — do not free.

```c
const char *weby_version(void);
```

#### `weby_amount_parse`

Parse a decimal amount string into integer wats (1 webcash = 100,000,000 wats).

```c
int32_t weby_amount_parse(
    const char *amount_str,   // e.g., "1.5"
    int64_t *out_wats         // receives 150000000
);
```

#### `weby_amount_format`

Format integer wats as a decimal string.

```c
int32_t weby_amount_format(
    int64_t wats,             // e.g., 150000000
    char **out_str            // receives "1.5" — free with weby_free_string()
);
```

#### `weby_free_string`

Free a string previously returned by webylib. Safe to call with NULL (no-op).

```c
void weby_free_string(char *ptr);
```

#### `weby_last_error_message`

Get the last error message for the current thread. Returns NULL if no error. Do **not** free the returned pointer.

```c
const char *weby_last_error_message(void);
```

---

## Language Bindings

### Python (ctypes)

```python
import ctypes

lib = ctypes.CDLL("./target/release/libwebylib.dylib")  # or .so / .dll

# Define function signatures
lib.weby_wallet_open.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
lib.weby_wallet_open.restype = ctypes.c_int32
lib.weby_wallet_balance.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_char_p)]
lib.weby_wallet_balance.restype = ctypes.c_int32
lib.weby_wallet_free.argtypes = [ctypes.c_void_p]
lib.weby_free_string.argtypes = [ctypes.c_char_p]
lib.weby_last_error_message.restype = ctypes.c_char_p

# Open wallet
wallet = ctypes.c_void_p()
rc = lib.weby_wallet_open(b"my_wallet.db", ctypes.byref(wallet))
if rc != 0:
    print(f"Error: {lib.weby_last_error_message().decode()}")
    exit(1)

# Get balance
balance = ctypes.c_char_p()
lib.weby_wallet_balance(wallet, ctypes.byref(balance))
print(f"Balance: {balance.value.decode()}")
lib.weby_free_string(balance)

# Clean up
lib.weby_wallet_free(wallet)
```

### Node.js (ffi-napi)

```javascript
const ffi = require('ffi-napi');
const ref = require('ref-napi');

const lib = ffi.Library('./target/release/libwebylib', {
  'weby_wallet_open':    ['int', ['string', ref.refType('pointer')]],
  'weby_wallet_balance': ['int', ['pointer', ref.refType('string')]],
  'weby_wallet_insert':  ['int', ['pointer', 'string']],
  'weby_wallet_pay':     ['int', ['pointer', 'string', 'string', ref.refType('string')]],
  'weby_wallet_free':    ['void', ['pointer']],
  'weby_free_string':    ['void', ['pointer']],
  'weby_last_error_message': ['string', []],
  'weby_version':        ['string', []],
});

// Open wallet
const walletPtr = ref.alloc('pointer');
const rc = lib.weby_wallet_open('my_wallet.db', walletPtr);
if (rc !== 0) throw new Error(lib.weby_last_error_message());
const wallet = walletPtr.deref();

// Get balance
const balancePtr = ref.alloc('string');
lib.weby_wallet_balance(wallet, balancePtr);
console.log(`Balance: ${balancePtr.deref()}`);

lib.weby_wallet_free(wallet);
```

### C# / .NET (P/Invoke)

```csharp
using System;
using System.Runtime.InteropServices;

public static class Webylib
{
    const string LIB = "webylib";

    [DllImport(LIB)] public static extern int weby_wallet_open(string path, out IntPtr wallet);
    [DllImport(LIB)] public static extern int weby_wallet_balance(IntPtr wallet, out IntPtr balance);
    [DllImport(LIB)] public static extern int weby_wallet_insert(IntPtr wallet, string webcash);
    [DllImport(LIB)] public static extern int weby_wallet_pay(IntPtr wallet, string amount, string memo, out IntPtr webcash);
    [DllImport(LIB)] public static extern int weby_wallet_check(IntPtr wallet);
    [DllImport(LIB)] public static extern void weby_wallet_free(IntPtr wallet);
    [DllImport(LIB)] public static extern void weby_free_string(IntPtr str);
    [DllImport(LIB)] public static extern IntPtr weby_last_error_message();
    [DllImport(LIB)] public static extern IntPtr weby_version();
}

// Usage:
IntPtr wallet;
int rc = Webylib.weby_wallet_open("my_wallet.db", out wallet);
if (rc != 0) {
    Console.WriteLine($"Error: {Marshal.PtrToStringAnsi(Webylib.weby_last_error_message())}");
    return;
}

IntPtr balancePtr;
Webylib.weby_wallet_balance(wallet, out balancePtr);
Console.WriteLine($"Balance: {Marshal.PtrToStringAnsi(balancePtr)}");
Webylib.weby_free_string(balancePtr);

Webylib.weby_wallet_free(wallet);
```

### Go (cgo)

```go
package main

/*
#cgo LDFLAGS: -L./target/release -lwebylib -lm -ldl -lpthread
#include <stdlib.h>

extern int weby_wallet_open(const char *path, void **out_wallet);
extern int weby_wallet_balance(const void *wallet, char **out_balance);
extern void weby_wallet_free(void *wallet);
extern void weby_free_string(char *ptr);
extern const char *weby_last_error_message();
*/
import "C"
import (
    "fmt"
    "unsafe"
)

func main() {
    var wallet unsafe.Pointer
    path := C.CString("my_wallet.db")
    defer C.free(unsafe.Pointer(path))

    rc := C.weby_wallet_open(path, &wallet)
    if rc != 0 {
        fmt.Printf("Error: %s\n", C.GoString(C.weby_last_error_message()))
        return
    }
    defer C.weby_wallet_free(wallet)

    var balance *C.char
    C.weby_wallet_balance(wallet, &balance)
    fmt.Printf("Balance: %s\n", C.GoString(balance))
    C.weby_free_string(balance)
}
```

### Swift

```swift
import Foundation

// Link against libwebylib.a (static) or libwebylib.dylib (dynamic)
// Add bridging header with webylib.h

var wallet: OpaquePointer?
let rc = weby_wallet_open("my_wallet.db", &wallet)
guard rc == 0, let w = wallet else {
    if let msg = weby_last_error_message() {
        print("Error: \(String(cString: msg))")
    }
    exit(1)
}

var balance: UnsafeMutablePointer<CChar>?
weby_wallet_balance(w, &balance)
if let b = balance {
    print("Balance: \(String(cString: b))")
    weby_free_string(b)
}

weby_wallet_free(w)
```

### Java (JNI / JNA)

```java
import com.sun.jna.Library;
import com.sun.jna.Native;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;

public interface Webylib extends Library {
    Webylib INSTANCE = Native.load("webylib", Webylib.class);

    int weby_wallet_open(String path, PointerByReference outWallet);
    int weby_wallet_balance(Pointer wallet, PointerByReference outBalance);
    int weby_wallet_insert(Pointer wallet, String webcash);
    void weby_wallet_free(Pointer wallet);
    void weby_free_string(Pointer ptr);
    String weby_last_error_message();
    String weby_version();
}

// Usage:
PointerByReference walletRef = new PointerByReference();
int rc = Webylib.INSTANCE.weby_wallet_open("my_wallet.db", walletRef);
if (rc != 0) {
    System.err.println("Error: " + Webylib.INSTANCE.weby_last_error_message());
    return;
}
Pointer wallet = walletRef.getValue();

PointerByReference balanceRef = new PointerByReference();
Webylib.INSTANCE.weby_wallet_balance(wallet, balanceRef);
System.out.println("Balance: " + balanceRef.getValue().getString(0));
Webylib.INSTANCE.weby_free_string(balanceRef.getValue());

Webylib.INSTANCE.weby_wallet_free(wallet);
```

### Kotlin (JNA)

```kotlin
import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import com.sun.jna.ptr.PointerByReference

interface Webylib : Library {
    fun weby_wallet_open(path: String, outWallet: PointerByReference): Int
    fun weby_wallet_balance(wallet: Pointer, outBalance: PointerByReference): Int
    fun weby_wallet_insert(wallet: Pointer, webcash: String): Int
    fun weby_wallet_free(wallet: Pointer)
    fun weby_free_string(ptr: Pointer)
    fun weby_last_error_message(): String?
    fun weby_version(): String

    companion object {
        val INSTANCE: Webylib = Native.load("webylib", Webylib::class.java)
    }
}

fun main() {
    val walletRef = PointerByReference()
    val rc = Webylib.INSTANCE.weby_wallet_open("my_wallet.db", walletRef)
    check(rc == 0) { "Error: ${Webylib.INSTANCE.weby_last_error_message()}" }
    val wallet = walletRef.value

    val balanceRef = PointerByReference()
    Webylib.INSTANCE.weby_wallet_balance(wallet, balanceRef)
    println("Balance: ${balanceRef.value.getString(0)}")
    Webylib.INSTANCE.weby_free_string(balanceRef.value)

    Webylib.INSTANCE.weby_wallet_free(wallet)
}
```
