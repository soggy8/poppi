# Contributing to Poppi Launcher

Thank you for your interest in contributing to Poppi Launcher! This document provides guidelines and instructions for contributing.

## How to Contribute

### Reporting Bugs

If you find a bug, please open an issue on GitHub with:

- A clear, descriptive title
- Steps to reproduce the bug
- Expected behavior
- Actual behavior
- Your system information (OS, GTK version, Rust version)
- Any relevant logs or error messages

### Suggesting Features

Feature suggestions are welcome! Please open an issue with:

- A clear description of the feature
- Use cases and examples
- Any implementation ideas (optional)

### Pull Requests

1. **Fork the repository**
2. **Create a feature branch** (`git checkout -b feature/amazing-feature`)
3. **Make your changes**
4. **Test your changes** (`cargo build --release` and test manually)
5. **Commit your changes** (`git commit -m 'Add some amazing feature'`)
   - Use clear, descriptive commit messages
   - Reference issue numbers if applicable
6. **Push to your branch** (`git push origin feature/amazing-feature`)
7. **Open a Pull Request**

### Code Style

- Follow Rust conventions (use `rustfmt` and `clippy`)
- Write clear, self-documenting code
- Add comments for complex logic
- Keep functions small and focused

### Testing

- Test your changes thoroughly before submitting
- Test on different Linux distributions if possible
- Ensure the code compiles without warnings

## Development Setup

1. Clone the repository:
```bash
git clone https://github.com/soggy8/poppi.git
cd poppi
```

2. Install dependencies (see README.md)

3. Build the project:
```bash
cargo build
```

4. Run in debug mode:
```bash
cargo run
```

## Areas for Contribution

- 🐛 Bug fixes
- ✨ New features
- 📚 Documentation improvements
- 🎨 UI/UX improvements
- ⚡ Performance optimizations
- 🌐 Internationalization (i18n)
- 🔧 Configuration enhancements
- 🧪 Testing improvements

## Questions?

Feel free to open an issue for questions or discussions!

