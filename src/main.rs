use crossterm::{
    cursor::{MoveLeft, MoveRight},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    queue,
    style::{Color, SetForegroundColor},
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use relative_path::RelativePathBuf;
use std::{
    collections::VecDeque,
    env,
    io::{stdin, stdout, Result, Write},
    path::{Path, PathBuf},
    process,
};
mod colors;
mod commands;

/// Function parse line to arguments, with support for quote enclosures
///
/// Include seperators will ensure no character of text is lost
fn parse_parts(text: &str, include_seperators: bool) -> VecDeque<CommandPart> {
    // i hate this code
    // too much logic
    let mut parts = VecDeque::new();
    parts.push_back(CommandPart {
        text: String::new(),
        part_type: CommandPartType::Keyword,
    });
    let mut last_char_was_backslash = false;
    let mut in_quote = false;
    for char in text.chars() {
        let last = parts.back_mut().unwrap();

        if char == '\\' {
            if include_seperators || last_char_was_backslash {
                last.text.insert(last.text.len(), char);
            }
            last_char_was_backslash = !last_char_was_backslash;
            continue;
        }
        if char == '"' && !last_char_was_backslash && (in_quote || last.text.is_empty()) {
            in_quote = !in_quote;
            if in_quote {
                last.part_type = CommandPartType::QuotesArg;
            }
            if !include_seperators {
                last_char_was_backslash = false;
                continue;
            }
        }
        if char == ' ' && !in_quote {
            if include_seperators {
                last.text.insert(last.text.len(), char);
            }
            parts.push_back(CommandPart {
                text: String::new(),
                part_type: CommandPartType::RegularArg,
            });
            last_char_was_backslash = false;
            continue;
        }
        if char == ';' && !in_quote && !last_char_was_backslash {
            last_char_was_backslash = false;
            if !last.text.is_empty() {
                parts.push_back(CommandPart {
                    text: String::from(char),
                    part_type: CommandPartType::Special,
                });
                parts.push_back(CommandPart {
                    text: String::new(),
                    part_type: CommandPartType::Keyword,
                });
                continue;
            }
            last.part_type = CommandPartType::Special;
            last.text.insert(last.text.len(), char);
            parts.push_back(CommandPart {
                text: String::new(),
                part_type: CommandPartType::Keyword,
            });
            continue;
        }
        last.text.insert(last.text.len(), char);
        last_char_was_backslash = false;
    }
    parts
}

fn remove_empty_parts(parts: VecDeque<CommandPart>) -> VecDeque<CommandPart> {
    let mut new = VecDeque::new();
    for part in parts {
        if part.text.is_empty() && !matches!(part.part_type, CommandPartType::QuotesArg) {
        } else {
            new.push_back(part);
        }
    }
    new
}

/// Replace substring in string (non case sensitive!!)
fn replace_case_insensitive(source: String, pattern: String, replace: String) -> String {
    let mut pattern_index = 0;
    let mut found_index = None;
    for (i, c) in source.chars().enumerate() {
        if pattern_index >= pattern.chars().count() {
            break;
        }
        if c.to_lowercase().collect::<String>()
            == pattern
                .chars()
                .nth(pattern_index)
                .unwrap()
                .to_lowercase()
                .collect::<String>()
        {
            if pattern_index == 0 {
                found_index = Some(i);
            }
            pattern_index += 1;
        } else if i == source.chars().count() - 1 || found_index.is_some() {
            found_index = None;
            pattern_index = 0;
        }
        if i == source.chars().count() - 1 && pattern_index < pattern.chars().count() {
            found_index = None;
            pattern_index = 0;
        }
    }
    match found_index {
        Some(index) => {
            let mut new = String::new();
            for (i, c) in source.chars().enumerate() {
                if i < index || i >= index + pattern.chars().count() {
                    new.insert(new.len(), c);
                } else if i == index {
                    new.insert_str(new.len(), &replace);
                }
            }
            new
        }
        None => source,
    }
}

fn list_dir(dir: &Path) -> Result<Vec<String>> {
    let contents = std::fs::read_dir(dir)?;
    let contents = contents
        .flatten()
        .map(|item| item.file_name().to_string_lossy().to_string())
        .collect();
    Ok(contents)
}

enum AbsoluteOrRelativePathBuf {
    Relative(RelativePathBuf),
    Absolute(PathBuf),
}

/// Autocomplete an input word to a relative path
fn autocomplete(current_word: &String, mut item_index: usize) -> Option<AbsoluteOrRelativePathBuf> {
    let mut valid: Vec<AbsoluteOrRelativePathBuf> = Vec::new();
    // check if is absolute
    let path = PathBuf::from(&current_word);
    if path.is_absolute() {
        let mut file_name = path.file_name()?.to_string_lossy().to_string();
        let absolute_parent = path.parent()?;
        let contents = list_dir(absolute_parent).ok()?;

        if env::consts::OS == "windows" {
            file_name = file_name.to_lowercase()
        };
        for item in contents {
            let item_in_maybe_lowercase = if env::consts::OS == "windows" {
                item.to_lowercase()
            } else {
                item.clone()
            };
            if item_in_maybe_lowercase.starts_with(&file_name) {
                valid.push(AbsoluteOrRelativePathBuf::Absolute(
                    absolute_parent.join(item),
                ));
            }
        }
    } else {
        let path = RelativePathBuf::from(&current_word);
        let cwd = std::env::current_dir().ok()?;
        let file_name = if env::consts::OS == "windows" {
            path.file_name()?.to_lowercase()
        } else {
            path.file_name()?.to_string()
        };

        let absolute = &path.to_logical_path(&cwd);
        let absolute_parent = absolute.parent()?;

        let relative_parent = path.parent()?;

        let contents = list_dir(absolute_parent).ok()?;
        for item in contents {
            let item_in_maybe_lowercase = if env::consts::OS == "windows" {
                item.to_lowercase()
            } else {
                item.clone()
            };
            if item_in_maybe_lowercase.starts_with(&file_name) {
                valid.push(AbsoluteOrRelativePathBuf::Relative(
                    relative_parent.join(item),
                ));
            }
        }
    }
    if !valid.is_empty() {
        item_index %= valid.len();
        for (index, item) in valid.into_iter().enumerate() {
            if index == item_index {
                return Some(item);
            }
        }
    }

    None
}

fn absolute_pathbuf_to_string(input: PathBuf) -> String {
    let mut parts: Vec<String> = vec![];
    for component in input.components() {
        match component {
            std::path::Component::Normal(path) => {
                parts.push(path.to_string_lossy().to_string());
            }
            std::path::Component::Prefix(prefix_component) => {
                parts.push(prefix_component.as_os_str().to_string_lossy().to_string());
            }
            _ => {}
        };
    }
    parts.join("/")
}

struct Shoe {
    history_path: String,
    history: Vec<String>,
    history_index: usize,
    running: bool,
    listening: bool,
    cwd: PathBuf,
    input_text: String,
    cursor_pos: usize,
    current_dir_contents: Vec<String>,
    autocomplete_cycle_index: Option<usize>,
    last_input_before_autocomplete: Option<String>,
}

impl Shoe {
    fn new(history_path: String) -> Result<Self> {
        let history_text =
            std::fs::read_to_string(&history_path).expect("Couldn't read ~/.shoehistory");
        let history: Vec<String> = history_text
            .split('\n')
            .filter_map(|line| {
                if line.trim().is_empty() {
                    None
                } else {
                    Option::<String>::Some(line.to_string())
                }
            })
            .collect();

        let history_index = history.len();
        let cwd = std::env::current_dir()?;
        let current_dir_contents = list_dir(&cwd)?;

        Ok(Shoe {
            history_path,
            history,
            history_index,
            running: false,
            listening: false,
            cwd,
            input_text: String::new(),
            cursor_pos: 0,
            current_dir_contents,
            last_input_before_autocomplete: None,
            autocomplete_cycle_index: None,
        })
    }
    /// Convert cwd to a string, also replacing home path with ~
    fn cwd_to_str(&self) -> Result<String> {
        let path = self
            .cwd
            .to_str()
            .ok_or(std::io::Error::other("Couldn't read path as string"))?
            .to_string();
        let home_path = shellexpand::tilde("~").to_string();

        if env::consts::OS == "windows" {
            // windows has case insensitive paths
            Ok(replace_case_insensitive(path, home_path, "~".to_string()))
        } else {
            Ok(path.replace(&home_path, "~"))
        }
    }
    fn update_cwd(&mut self) -> Result<()> {
        self.cwd = std::env::current_dir()?;
        self.current_dir_contents = list_dir(&self.cwd)?;
        Ok(())
    }
    fn execute_command(
        &mut self,
        keyword: &String,
        context: commands::CommandContext,
    ) -> Result<()> {
        let result = commands::execute_command(keyword, &context);
        match result {
            commands::CommandResult::NotACommand => {}
            commands::CommandResult::Exit => {
                self.listening = false;
                self.running = false;
            }
            commands::CommandResult::UpdateCwd => self.update_cwd()?,
            _ => {}
        }
        if !matches!(result, commands::CommandResult::NotACommand) {
            queue!(stdout(), SetForegroundColor(Color::Reset))?;
            return Ok(());
        }

        // if on windows, also try running the keyword with the .bat and .cmd extensions if the regular fails
        let keywords: Vec<String>;
        if env::consts::OS == "windows" {
            if !keyword.contains(".") {
                keywords = vec![
                    keyword.to_string(),
                    keyword.to_string() + ".bat",
                    keyword.to_string() + ".cmd",
                ]
            } else {
                keywords = vec![keyword.to_string()];
            }
        } else {
            keywords = vec![keyword.to_string()];
        }

        for keyword in keywords {
            let mut command = process::Command::new(&keyword);
            command.args(context.args);
            let process = command.spawn();
            if let Ok(mut process) = process {
                process.wait()?;
                return Ok(());
            }
        }
        let message = format!("file/command '{}' not found! :(", keyword);
        Err(std::io::Error::other(message))
    }
    fn write_char(&mut self, new_char: char) {
        if self.input_text.chars().count() == self.cursor_pos {
            self.input_text.insert(self.input_text.len(), new_char);
            return;
        } else if self.cursor_pos == 0 {
            self.input_text.insert(0, new_char);
            return;
        }
        let mut new = String::new();
        for (index, char) in self.input_text.chars().enumerate() {
            new.insert(new.len(), char);
            if index == self.cursor_pos - 1 {
                new.insert(new.len(), new_char);
            }
        }
        self.input_text = new;
    }
    fn delete_char(&mut self) {
        let mut new = String::new();
        for (index, char) in self.input_text.chars().enumerate() {
            if index != self.cursor_pos {
                new.insert(new.len(), char);
            }
        }
        self.input_text = new;
    }
    fn get_word_at_cursor(&self) -> Option<(usize, CommandPart)> {
        let mut counter = 0;
        for (index, part) in parse_parts(&self.input_text, false).into_iter().enumerate() {
            counter += part.text.chars().count() + 1;
            if counter >= self.cursor_pos {
                return Some((index, part));
            }
        }
        None
    }
    fn handle_key_press(&mut self, event: Event) -> Result<()> {
        if let Event::Key(key_event) = event {
            if key_event.kind != KeyEventKind::Press {
                return Ok(());
            }
            let mut reset_autocomplete_cycle = true;
            match key_event.code {
                KeyCode::Enter => {
                    self.listening = false;
                }
                KeyCode::Char(char) => {
                    if key_event.modifiers.contains(KeyModifiers::CONTROL) && char == 'c' {
                        self.input_text = String::new();
                        self.listening = false;
                    } else {
                        self.write_char(char);
                        self.cursor_pos += 1;
                    }
                }
                KeyCode::Tab => 'tab: {
                    reset_autocomplete_cycle = false;
                    if self.input_text.is_empty() {
                        break 'tab;
                    }
                    if let Some(last_input) = &self.last_input_before_autocomplete {
                        self.cursor_pos -= self.input_text.len().saturating_sub(last_input.len());
                        self.input_text = last_input.to_string();
                        self.autocomplete_cycle_index =
                            Some(self.autocomplete_cycle_index.unwrap() + 1);
                    } else {
                        self.autocomplete_cycle_index = Some(0);
                        self.last_input_before_autocomplete = Some(self.input_text.to_string());
                    }
                    let mut words = parse_parts(&self.input_text, true);
                    let Some((word_index, word)) = self.get_word_at_cursor() else {
                        break 'tab;
                    };
                    let ends_with_quote =
                        matches!(words[word_index].part_type, CommandPartType::QuotesArg)
                            && words[word_index].text.ends_with('"');
                    let starts_with_quote =
                        matches!(words[word_index].part_type, CommandPartType::QuotesArg);
                    let ends_with_space = words[word_index].text.ends_with(' ');
                    words.remove(word_index);

                    let autocompletion =
                        autocomplete(&word.text, self.autocomplete_cycle_index.unwrap());
                    let Some(autocompletion) = autocompletion else {
                        break 'tab;
                    };
                    let autocompletion_is_dir: bool;
                    let mut autocompletion_string: String;
                    match autocompletion {
                        AbsoluteOrRelativePathBuf::Relative(relative) => {
                            autocompletion_string = relative.to_string();
                            autocompletion_is_dir = relative.to_logical_path(&self.cwd).is_dir();
                        }
                        AbsoluteOrRelativePathBuf::Absolute(absolute) => {
                            autocompletion_is_dir = absolute.is_dir();
                            autocompletion_string = absolute_pathbuf_to_string(absolute);
                        }
                    }
                    if autocompletion_is_dir {
                        autocompletion_string += "/";
                    }
                    if autocompletion_string.contains(' ') && !starts_with_quote {
                        autocompletion_string = String::from("\"") + &autocompletion_string;
                        if word_index != words.len() {
                            autocompletion_string += "\"";
                        }
                    }
                    self.cursor_pos +=
                        autocompletion_string.chars().count() - word.text.chars().count();
                    if starts_with_quote {
                        autocompletion_string = String::from("\"") + &autocompletion_string;
                    }
                    if ends_with_quote {
                        autocompletion_string += "\"";
                    }
                    if ends_with_space {
                        autocompletion_string += " ";
                    }
                    let mut new = String::new();
                    for (index, word) in words.iter().enumerate() {
                        if word_index == index {
                            new += &autocompletion_string;
                        }
                        new += &word.text;
                    }
                    if word_index == words.len() {
                        new += &autocompletion_string;
                    }
                    self.input_text = new;
                }
                KeyCode::Delete => {
                    self.delete_char();
                }
                KeyCode::Backspace => {
                    if self.cursor_pos > 0 {
                        self.cursor_pos -= 1;
                        self.delete_char();
                    }
                }
                KeyCode::Up => {
                    if !self.history.is_empty() {
                        if self.history_index > 0 {
                            self.history_index -= 1;
                        }
                        self.input_text = self.history[self.history_index].clone();
                        self.cursor_pos = self.input_text.chars().count();
                    }
                }
                KeyCode::Down => {
                    if !self.history.is_empty() {
                        if self.history_index < self.history.len() {
                            self.history_index += 1;
                        }
                        if self.history_index < self.history.len() {
                            self.input_text = self.history[self.history_index].clone();
                            self.cursor_pos = self.input_text.chars().count();
                        } else {
                            self.input_text = String::new();
                            self.cursor_pos = 0;
                        }
                    }
                }
                KeyCode::Right => {
                    self.cursor_pos += 1;
                    if self.cursor_pos > self.input_text.chars().count() {
                        self.cursor_pos = self.input_text.chars().count();
                    }
                }
                KeyCode::Left => {
                    self.cursor_pos = self.cursor_pos.saturating_sub(1);
                }
                KeyCode::Home => {
                    self.cursor_pos = 0;
                }
                KeyCode::End => {
                    self.cursor_pos = self.input_text.chars().count();
                }
                _ => {}
            }
            if reset_autocomplete_cycle {
                self.autocomplete_cycle_index = None;
                self.last_input_before_autocomplete = None;
            }
            self.update()?;
        }
        Ok(())
    }
    /// Prints current inputted text with color highlighting
    fn print_text(&self) -> Result<()> {
        let parts = parse_parts(&self.input_text, true);
        for part in parts {
            let color = match part.part_type {
                CommandPartType::Keyword => colors::PRIMARY_COLOR,
                CommandPartType::QuotesArg => colors::SECONDARY_COLOR,
                CommandPartType::RegularArg => Color::White,
                CommandPartType::Special => colors::SECONDARY_COLOR,
            };
            queue!(stdout(), SetForegroundColor(color))?;
            print!("{}", part.text);
            queue!(stdout(), SetForegroundColor(Color::Reset))?;
        }
        Ok(())
    }
    fn update(&self) -> Result<()> {
        queue!(stdout(), Clear(ClearType::UntilNewLine))?;
        self.print_text()?;
        if self.input_text.chars().count() != 0 {
            queue!(stdout(), MoveLeft((self.input_text.chars().count()) as u16))?;
        }

        if self.cursor_pos != 0 {
            queue!(stdout(), MoveRight(self.cursor_pos as u16))?;
        }

        stdout().flush()?;

        if self.cursor_pos != 0 {
            queue!(stdout(), MoveLeft(self.cursor_pos as u16))?;
        }

        Ok(())
    }
    fn start(&mut self) -> Result<()> {
        self.running = true;
        while self.running {
            let command = &self.listen()?;
            if command.is_empty() {
                self.history_index = self.history.len();
                continue;
            }
            let mut should_store_history = true;
            if let Some(last_input) = self.history.last() {
                if last_input == command {
                    should_store_history = false;
                }
            }

            if should_store_history {
                self.history.push(command.clone());
                std::fs::write(&self.history_path, self.history.join("\n"))?;
            }
            self.history_index = self.history.len();

            let parts = remove_empty_parts(parse_parts(command, false));
            let mut commands: Vec<VecDeque<CommandPart>> = vec![VecDeque::new()];

            for part in parts {
                let current_command = commands.last_mut().unwrap();
                if let CommandPartType::Special = part.part_type {
                    commands.push(VecDeque::new());
                } else {
                    current_command.push_back(part);
                }
            }
            for command in commands {
                let mut args: VecDeque<&String> = command.iter().map(|item| &item.text).collect();
                let keyword = args.pop_front();
                if let Some(keyword) = keyword {
                    let context = commands::CommandContext {
                        args: &args,
                        stdout: stdout(),
                        _stdin: stdin(),
                    };

                    let result = self.execute_command(keyword, context);
                    if let Err(error) = result {
                        let _ = queue!(stdout(), SetForegroundColor(colors::ERR_COLOR));
                        println!("{}", error);
                    }

                    queue!(stdout(), SetForegroundColor(Color::Reset))?;
                }
            }
        }
        Ok(())
    }
    fn listen(&mut self) -> Result<String> {
        enable_raw_mode()?;
        self.listening = true;

        queue!(stdout(), SetForegroundColor(colors::SECONDARY_COLOR))?;
        print!("[");
        queue!(stdout(), SetForegroundColor(Color::White))?;
        print!("{}", self.cwd_to_str()?);
        queue!(stdout(), SetForegroundColor(colors::SECONDARY_COLOR))?;
        print!("]> ");

        stdout().flush()?;
        while self.listening {
            self.handle_key_press(event::read()?)?;
        }
        if self.input_text.chars().count() != 0 {
            queue!(stdout(), MoveRight(self.input_text.chars().count() as u16))?;
        }
        println!();
        disable_raw_mode()?;
        let text = self.input_text.clone();
        self.input_text = String::new();
        self.cursor_pos = 0;
        Ok(text)
    }
}

enum CommandPartType {
    Keyword,
    QuotesArg,
    RegularArg,
    Special,
}
struct CommandPart {
    text: String,
    part_type: CommandPartType,
}

fn main() {
    queue!(stdout(), SetForegroundColor(colors::SECONDARY_COLOR)).unwrap();
    print!("shoe ");
    queue!(stdout(), SetForegroundColor(Color::White)).unwrap();
    print!("[v{}]\n\n", env!("CARGO_PKG_VERSION"));
    stdout().flush().unwrap();

    let path = shellexpand::tilde("~/.shoehistory").to_string();
    if std::fs::metadata(&path).is_err() {
        std::fs::write(&path, "").expect("Couldn't create ~/.shoehistory");
    }

    let mut shoe = Shoe::new(path).unwrap();
    shoe.start().unwrap();
}
