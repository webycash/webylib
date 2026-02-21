# Contributing to Webcash Rust Wallet

Thank you for your interest in contributing to the Webcash Rust wallet! This document provides guidelines and information for contributors.

## Development Setup

### Prerequisites

- **Rust**: Version 1.70 or later
- **Cargo**: Rust's package manager (comes with Rust)
- **SQLite3**: Development libraries
- **OpenSSL**: Development libraries
- **Git**: Version control system

### Installation

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Clone the repository
git clone <repository-url>
cd webylib

# Build the project
cargo build

# Run tests
cargo test
```

## Development Workflow

### 1. Choose an Issue

- Check the [GitHub Issues](../../issues) for tasks to work on
- Look for issues labeled `good first issue` or `help wanted`
- Comment on the issue to indicate you're working on it

### 2. Create a Branch

```bash
# Create and switch to a new branch
git checkout -b feature/your-feature-name
# or
git checkout -b fix/issue-number-description
```

### 3. Make Changes

- Follow the existing code style and patterns
- Write tests for new functionality
- Update documentation as needed
- Ensure all tests pass

### 4. Commit Changes

```bash
# Stage your changes
git add .

# Commit with a descriptive message
git commit -m "feat: add new feature description

- What was changed
- Why it was changed
- Any breaking changes
"

# Push to your branch
git push origin feature/your-feature-name
```

### 5. Create Pull Request

- Go to the repository on GitHub
- Click "New Pull Request"
- Select your branch as the source
- Fill out the pull request template
- Request review from maintainers

## Code Style

### Rust Style Guidelines

This project follows the official Rust style guidelines:

- Use `rustfmt` for code formatting
- Use `clippy` for linting
- Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)

### Formatting

```bash
# Format code
cargo fmt

# Check for style issues
cargo clippy

# Fix common issues automatically
cargo fix
```

### Naming Conventions

- **Types/Structs/Enums**: `PascalCase`
- **Functions/Methods**: `snake_case`
- **Constants**: `SCREAMING_SNAKE_CASE`
- **Modules**: `snake_case`

### Documentation

- All public APIs must have documentation comments (`///`)
- Documentation should follow the [Rust documentation guidelines](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html)
- Include code examples where appropriate

```rust
/// Brief description of what this function does.
///
/// # Arguments
///
/// * `param1` - Description of param1
/// * `param2` - Description of param2
///
/// # Returns
///
/// Description of return value
///
/// # Examples
///
/// ```
/// let result = my_function(1, 2);
/// assert_eq!(result, 3);
/// ```
pub fn my_function(param1: i32, param2: i32) -> i32 {
    // implementation
}
```

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run benchmarks (if available)
cargo bench
```

### Writing Tests

- Write unit tests for all public functions
- Use descriptive test names
- Test both success and failure cases
- Use `#[test]` attribute for unit tests
- Use `#[tokio::test]` for async tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_function_success() {
        let result = my_function(1, 2);
        assert_eq!(result, 3);
    }

    #[test]
    fn test_my_function_error() {
        let result = my_function(-1, 2);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_async_function() {
        let result = async_function().await;
        assert!(result.is_ok());
    }
}
```

### Test Coverage

```bash
# Install cargo-llvm-cov
cargo install cargo-llvm-cov

# Generate coverage report
cargo llvm-cov

# Open HTML report
cargo llvm-cov --open
```

## Security Considerations

### Sensitive Data Handling

- Use `SecureString` for sensitive data
- Implement `Drop` trait for secure cleanup
- Avoid logging sensitive information
- Use `zeroize` crate for sensitive memory

### Cryptographic Operations

- Use well-established cryptographic libraries
- Validate all inputs to cryptographic functions
- Implement proper error handling
- Follow cryptographic best practices

### Input Validation

- Validate all user inputs
- Use safe parsing methods
- Implement bounds checking
- Handle edge cases properly

## Commit Message Guidelines

Follow conventional commit format:

```
type(scope): description

[optional body]

[optional footer]
```

### Types

- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Test additions/changes
- `chore`: Maintenance tasks

### Examples

```
feat(wallet): add support for HD key derivation

fix(amount): handle overflow in amount parsing

docs(api): update wallet API documentation

test(crypto): add tests for SHA256 implementation
```

## Pull Request Process

### Before Submitting

1. **Update Documentation**: Ensure all changes are documented
2. **Run Tests**: All tests must pass
3. **Code Review**: Self-review your code
4. **Security Check**: Ensure no sensitive data is committed

### Pull Request Template

Please fill out the pull request template with:

- Description of changes
- Testing instructions
- Breaking changes (if any)
- Related issues

### Review Process

1. Automated checks (CI/CD)
2. Code review by maintainers
3. Testing and validation
4. Merge approval

## Code of Conduct

This project follows a code of conduct to ensure a welcoming environment for all contributors. By participating, you agree to:

- Be respectful and inclusive
- Focus on constructive feedback
- Accept responsibility for mistakes
- Show empathy towards other contributors
- Help create a positive community

## Getting Help

- **Issues**: Use GitHub Issues for bugs and feature requests
- **Discussions**: Use GitHub Discussions for questions and general discussion
- **Documentation**: Check the docs/ folder for detailed information

## Recognition

Contributors will be recognized in:
- GitHub repository contributors list
- Changelog for significant contributions
- Project documentation

Thank you for contributing to the Webcash Rust wallet! 🚀
