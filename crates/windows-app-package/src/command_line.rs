//! 按 Windows C 运行库解析命令行的规则拼命令行，和标准库 `Command` 的拼法一样。

use std::io;

const SPACE: u16 = b' ' as u16;
const TAB: u16 = b'\t' as u16;
const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;

/// 程序名总是加引号；程序名不按反斜杠转义解析，所以里面不能有双引号。
pub(crate) fn append_program(line: &mut Vec<u16>, program: &[u16]) -> io::Result<()> {
    if program.contains(&QUOTE) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "program path contains a double quote",
        ));
    }
    line.push(QUOTE);
    line.extend_from_slice(program);
    line.push(QUOTE);
    Ok(())
}

/// 空的、带空格或制表符的参数加引号；双引号前面的反斜杠翻倍再转义双引号，结尾的反斜杠在引号前翻倍。
pub(crate) fn append_argument(line: &mut Vec<u16>, argument: &[u16]) {
    let quote = argument.is_empty() || argument.iter().any(|&unit| unit == SPACE || unit == TAB);
    if quote {
        line.push(QUOTE);
    }
    let mut backslashes = 0;
    for &unit in argument {
        if unit == BACKSLASH {
            backslashes += 1;
        } else {
            if unit == QUOTE {
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes + 1));
            }
            backslashes = 0;
        }
        line.push(unit);
    }
    if quote {
        line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
        line.push(QUOTE);
    }
}

#[cfg(test)]
mod tests {
    use super::{append_argument, append_program};

    fn argument(text: &str) -> String {
        let mut line = Vec::new();
        append_argument(&mut line, &text.encode_utf16().collect::<Vec<_>>());
        String::from_utf16(&line).expect("拼出来的还是 UTF-16")
    }

    #[test]
    fn 普通参数原样() {
        assert_eq!(argument("--background"), "--background");
        assert_eq!(argument(r"C:\dir\file"), r"C:\dir\file");
    }

    #[test]
    fn 空参数和带空格的参数加引号() {
        assert_eq!(argument(""), r#""""#);
        assert_eq!(argument("a b"), r#""a b""#);
        assert_eq!(argument("a\tb"), "\"a\tb\"");
    }

    #[test]
    fn 双引号和它前面的反斜杠要转义() {
        assert_eq!(argument(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(argument(r#"a\"b"#), r#"a\\\"b"#);
    }

    #[test]
    fn 加了引号时结尾的反斜杠翻倍() {
        assert_eq!(argument(r"C:\with space\"), r#""C:\with space\\""#);
        assert_eq!(argument(r"C:\no-space\"), r"C:\no-space\");
    }

    #[test]
    fn 程序名总加引号且不许带双引号() {
        let mut line = Vec::new();
        append_program(
            &mut line,
            &r"C:\Program Files\app.exe"
                .encode_utf16()
                .collect::<Vec<_>>(),
        )
        .expect("普通路径可用");
        assert_eq!(
            String::from_utf16(&line).expect("UTF-16"),
            r#""C:\Program Files\app.exe""#
        );
        assert!(
            append_program(&mut Vec::new(), &"a\"b".encode_utf16().collect::<Vec<_>>()).is_err()
        );
    }
}
