```markdown
# rmk-cornix Development Patterns

> Auto-generated skill from repository analysis

## Overview
This skill covers the core development patterns and workflows for the `rmk-cornix` Rust codebase. It outlines coding conventions, file organization, and the main workflow for updating status LED colors. Whether you're contributing new features or maintaining the code, this guide will help you follow established practices for consistency and clarity.

## Coding Conventions

### File Naming
- Use **camelCase** for file names.
  - Example: `statusLed.rs`, `ws2812.rs`

### Import Style
- Use **relative imports** within the crate.
  - Example:
    ```rust
    mod ws2812;
    use crate::ws2812::set_led_color;
    ```

### Export Style
- Use **named exports** for functions and modules.
  - Example:
    ```rust
    pub fn set_led_color(profile: Profile) { ... }
    ```

### Commit Messages
- Follow **conventional commit** format.
- Prefixes: `feat`, `perf`
- Example:
  ```
  feat: add support for custom LED profiles
  perf: optimize color update routine
  ```

## Workflows

### Update Status LED Colors
**Trigger:** When someone wants to change the status LED color scheme for different profiles or states.  
**Command:** `/update-led-colors`

1. **Edit color logic:**
   - Open `src/ws2812.rs`.
   - Locate the section defining LED color values or logic.
   - Update the color palette as needed.
     ```rust
     // Example: Update color for "active" profile
     const ACTIVE_COLOR: (u8, u8, u8) = (0, 255, 0); // Green
     ```
2. **Document changes:**
   - Open `README.md`.
   - Update the documentation to reflect the new color palette and explain usage.
     ```
     ## Status LED Colors
     - Active: Green
     - Idle: Blue
     - Error: Red
     ```
3. **Commit your changes:**
   - Use a conventional commit message, e.g.:
     ```
     feat: update status LED color palette
     ```

## Testing Patterns

- **Testing framework:** Unknown (no framework detected).
- **Test file pattern:** Files named with `*.test.*`.
  - Example: `ws2812.test.rs`
- **Test structure:** Standard Rust test modules.
  - Example:
    ```rust
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_set_led_color() {
            // test logic here
        }
    }
    ```

## Commands

| Command             | Purpose                                                      |
|---------------------|--------------------------------------------------------------|
| /update-led-colors  | Update the status LED color palette and document the changes |
```
