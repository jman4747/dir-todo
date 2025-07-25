/// dir map is "path" => "hash of path"
use std::{
    env::{current_dir, home_dir},
    fs::{OpenOptions, create_dir, read_to_string, rename},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use argh::FromArgs;
use inquire::Confirm;
use tinyvec::TinyVec;

const TODO_DIR_NAME: &str = "todo";
const DIR_MAP_NAME: &str = "dirmap.tsv";
const DIR_MAP_NEW_NAME: &str = "dirmap.new.tsv";
const COL_SEP_CH: char = '\t';
const ACTIVE_TODO: &str = "[ ]";
const DONE_TODO: &str = "[✓]";

fn main() {
    let todo: Todo = argh::from_env();
    let mut todo_dir = {
        let mut todo_dir = home_dir().expect("get home dir");
        todo_dir.as_mut_os_string().reserve(256);
        todo_dir.push(TODO_DIR_NAME);
        todo_dir
    };

    let pwd = current_dir()
        .map(|pwd| {
            pwd.to_str()
                .expect("pwd is utf-8 string")
                .to_string()
                .into_boxed_str()
        })
        .expect("get pwd");

    //create todo dir in home
    if !todo_dir.exists() {
        create_dir(&todo_dir).expect("create todo");
    }

    let mut dir_map_handle = with_pushed(&mut todo_dir, DIR_MAP_NAME, |path| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .append(true)
            .create(true)
            .open(path)
    })
    .expect("open dir map");

    let mut dir_map_buf = String::with_capacity(
        dir_map_handle
            .metadata()
            .map(|m| m.len() as usize)
            .unwrap_or(4096),
    );

    dir_map_handle
        .read_to_string(&mut dir_map_buf)
        .expect("load all of dir map");

    drop(dir_map_handle);

    let cmd = todo.cmd.unwrap_or_default();
    match cmd {
        Command::New(new_todo) => {
            create_new_todo(new_todo, &pwd, &mut todo_dir, &mut dir_map_buf);
        }
        Command::List(all) => {
            if all.all {
                list_todos_all(&mut dir_map_buf, &mut todo_dir);
            } else {
                list_todos_pwd(&mut dir_map_buf, &pwd, &mut todo_dir);
            }
        }
        Command::Update(update) => {
            update_todo(update, &mut dir_map_buf, &pwd, &mut todo_dir);
        }
        Command::Delete(delete_todo_id) => {
            delete_todo(delete_todo_id, &mut dir_map_buf, &pwd, &mut todo_dir);
        }
        Command::Done(done) => {
            mark_done(done, &mut dir_map_buf, &pwd, &mut todo_dir);
        }
        Command::Active(active) => {
            mark_active(active, &mut dir_map_buf, &pwd, &mut todo_dir);
        }
    }
}

#[derive(Debug, FromArgs, PartialEq)]
/// Directory mapped TODO
struct Todo {
    #[argh(subcommand)]
    cmd: Option<Command>,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
enum Command {
    New(NewTodo),
    List(ListTodo),
    Update(UpdateTodo),
    Delete(DeleteTodoId),
    Done(Done),
    Active(Active),
}

impl Default for Command {
    fn default() -> Self {
        Self::List(ListTodo::default())
    }
}

#[derive(FromArgs, PartialEq, Debug)]
/// Create a todo.
#[argh(subcommand, name = "new")]
struct NewTodo {
    #[argh(positional)]
    text: String,
}

impl NewTodo {
    // fn active_record_buff_size(&self) -> usize {
    //     self.text.len() + 2 + 1 + ACTIVE_TODO.len() + 2
    // }

    fn io_write_as_active(&self, buf: &mut impl std::io::Write, id: u64) -> std::io::Result<()> {
        writeln!(
            buf,
            "{id}{COL_SEP_CH}{}{COL_SEP_CH}{ACTIVE_TODO}",
            &self.text
        )
    }
}

#[derive(FromArgs, PartialEq, Debug)]
/// Mark a todo done.
#[argh(subcommand, name = "done")]
struct Done {
    #[argh(positional)]
    /// index of the todo (in this directory) to mark as done
    index: u64,
}

#[derive(FromArgs, PartialEq, Debug)]
/// Mark a todo active (not done).
#[argh(subcommand, name = "active")]
struct Active {
    #[argh(positional)]
    /// index of the todo (in this directory) to mark as active
    index: u64,
}

#[derive(FromArgs, PartialEq, Debug)]
/// List todos.
#[argh(subcommand, name = "list")]
struct ListTodo {
    #[argh(switch, short = 'a')]
    /// list all Todos regardless of directory
    all: bool,
}

impl Default for ListTodo {
    fn default() -> Self {
        Self { all: false }
    }
}

#[derive(FromArgs, PartialEq, Debug)]
/// Update a todo.
#[argh(subcommand, name = "update")]
struct UpdateTodo {
    #[argh(positional)]
    /// todo ID number
    id: u64,
    #[argh(positional)]
    /// new text of todo
    new_text: String,
}

#[derive(FromArgs, PartialEq, Debug)]
/// Delete a todo.
#[argh(subcommand, name = "delete")]
struct DeleteTodoId {
    #[argh(positional)]
    /// todo number
    id: u64,
}

fn calculate_hash<T: Hash>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

fn with_pushed<P, F, Out>(buf: &mut PathBuf, to_push: P, mut f: F) -> Out
where
    P: AsRef<Path>,
    F: FnMut(&Path) -> Out,
{
    buf.push(to_push);
    let out = f(buf.as_path());
    buf.pop();
    out
}

// fn with_pushed_and_ext<P, F, Out, E>(buf: &mut PathBuf, to_push: P, ext: E, mut f: F) -> Out
// where
//     P: AsRef<Path>,
//     F: FnMut(&Path) -> Out,
//     E: AsRef<OsStr>,
// {
//     buf.push(to_push);
//     buf.set_extension(ext);
//     let out = f(buf.as_path());
//     buf.pop();
//     out
// }

fn save_dir_map(todo_path: &mut PathBuf, dir_map_buf: &mut String) -> std::io::Result<()> {
    with_pushed(todo_path, DIR_MAP_NEW_NAME, |path| {
        let mut handle = OpenOptions::new()
            .read(true)
            .write(true)
            .append(true)
            .create(true)
            .open(path)?;
        handle.write_all(dir_map_buf.as_bytes())
    })?;
    let old = with_pushed(todo_path, DIR_MAP_NAME, |path| Box::from(path));
    with_pushed(todo_path, DIR_MAP_NEW_NAME, |path| rename(path, &old))
}

fn prompt_delete_active() -> bool {
    let ans = Confirm::new("This todo is active. Are you sure you want to delete it?")
        .with_default(false)
        .prompt();

    match ans {
        Ok(true) => true,
        Ok(false) => false,
        Err(_) => {
            println!("Error with questionnaire, try again later");
            false
        }
    }
}

fn create_new_todo(new_todo: NewTodo, pwd: &str, todo_dir: &mut PathBuf, dir_map_buf: &mut String) {
    // reject todo with newlines or tabs

    let text = new_todo.text.as_str();
    if reject_nl_and_tab(text) {
        return;
    }

    let dir_map_entries: Vec<(&str, &str)> = dir_map_buf
        .lines()
        .map(|line| {
            let mut cols = line.split(COL_SEP_CH);
            (
                // key (dir)
                cols.next().expect("read key"),
                // value (number)
                cols.next().expect("read todo file name"),
            )
        })
        .collect();
    let pwd_todo_map_entry = dir_map_entries.iter().find(|(k, _v)| **k == *pwd);
    let new_id = match pwd_todo_map_entry {
        Some((_path, index)) => {
            // dir_map_entries.push((Some(pwd), Some(path_hash)));
            // create file
            let (todo_file_existed, mut todo_file_handle) = with_pushed(todo_dir, index, |path| {
                (
                    path.is_file(),
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .append(true)
                        .open(path)
                        .expect("open todo"),
                )
            });
            let mut todo_buf = String::with_capacity(
                todo_file_handle
                    .metadata()
                    .map(|m| m.len() as usize + 1024)
                    .unwrap_or(4096),
            );
            todo_file_handle
                .read_to_string(&mut todo_buf)
                .expect("read todo file");

            let next_id: Option<u64> = if todo_file_existed {
                let mut next_id = 0;
                let mut same = None::<u64>;
                for line in todo_buf.lines() {
                    let mut columns = line.split(COL_SEP_CH);
                    let old_id = columns
                        .next()
                        .and_then(|old_id_str| old_id_str.parse::<u64>().ok())
                        .unwrap_or(0);
                    if old_id >= next_id {
                        next_id = old_id + 1
                    }
                    let old_text = columns.next();
                    if let Some(old) = old_text {
                        if let None = same {
                            if old == &new_todo.text {
                                same = Some(old_id)
                            }
                        }
                    }
                }
                match same {
                    Some(old_id) => {
                        println!(
                            "the todo: \"{}\" already exists at id: {old_id}",
                            new_todo.text
                        );
                        None
                    }
                    None => Some(next_id),
                }
            } else {
                Some(0)
            };

            if let Some(id) = next_id {
                new_todo
                    .io_write_as_active(&mut todo_file_handle, id)
                    .expect("write new todo to file");
            }

            next_id
        }
        _ => {
            let mut out_buf = String::with_capacity(20 + 4);
            use std::fmt::Write as _;
            let path_hash = calculate_hash(&pwd);
            write!(&mut out_buf, "{path_hash}.tsv").unwrap();
            // create file
            let mut todo_file_handle = with_pushed(todo_dir, &out_buf, |path| {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .append(true)
                    .open(path)
            })
            .expect("open todo");

            new_todo
                .io_write_as_active(&mut todo_file_handle, 0)
                .expect("write new todo to file");

            write!(dir_map_buf, "{pwd}\t{}\n", out_buf).unwrap();
            save_dir_map(todo_dir, dir_map_buf).expect("write new entry to dir map");
            Some(0)
        }
    };
    if let Some(new_id) = new_id {
        println!("added todo: \"{}\" at ID: {new_id}", &new_todo.text);
    }
}

fn update_todo(update: UpdateTodo, dir_map_buf: &mut String, pwd: &str, todo_dir: &mut PathBuf) {
    use std::fmt::Write;

    let text = update.new_text.as_str();
    if reject_nl_and_tab(text) {
        return;
    }

    let pwd_todo_map_entry = dir_map_entries(&dir_map_buf).find(|(k, _v)| **k == *pwd);
    match pwd_todo_map_entry {
        Some((_path, index)) => {
            // file must already exist
            let todo_file_handle = with_pushed(todo_dir, index, |path| {
                if !path.is_file() {
                    eprintln!("there are no todos to edit @: {path:?}");
                    return None;
                }
                Some(
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .append(false)
                        .truncate(false)
                        .open(path)
                        .expect("open todo"),
                )
            });
            let mut todo_file_handle = match todo_file_handle {
                Some(tfh) => tfh,
                None => return,
            };
            let todo_file_len = todo_file_handle
                .metadata()
                .map(|m| m.len() as usize)
                .unwrap_or(4096);
            let mut todo_buf = String::with_capacity(todo_file_len);
            todo_file_handle
                .read_to_string(&mut todo_buf)
                .expect("read todo file");

            // is there a todo at that ID?
            let existing_record = todo_buf.lines().enumerate().find_map(|(idx, line)| {
                line.split_once(COL_SEP_CH).and_then(|(id_str, rest)| {
                    rest.split_once(COL_SEP_CH).and_then(|(_old_txt, status)| {
                        if id_str.parse::<u64>().ok().is_some_and(|id| id == update.id) {
                            Some((idx, id_str, status))
                        } else {
                            None
                        }
                    })
                })
            });
            let existing_record = match existing_record {
                Some(er) => er,
                None => {
                    eprintln!("no record @ ID {} and path \"{}\"", update.id, pwd);
                    return;
                }
            };

            // resuse dir map buf for output of the todo file
            dir_map_buf.clear();
            if let Some(additional) =
                (todo_file_len + update.new_text.len()).checked_sub(dir_map_buf.capacity())
            {
                dir_map_buf.reserve(additional);
            }

            todo_file_handle
                .seek(SeekFrom::Start(0))
                .expect("set write position to star of todo file");

            // re-write todos with the update
            for (og_idx, og_line) in todo_buf.lines().enumerate() {
                if og_idx == existing_record.0 {
                    writeln!(
                        dir_map_buf,
                        "{}{COL_SEP_CH}{}{COL_SEP_CH}{}",
                        existing_record.1, update.new_text, existing_record.2
                    )
                    .unwrap()
                } else {
                    writeln!(dir_map_buf, "{og_line}").unwrap();
                }
            }
            todo_file_handle
                .set_len(dir_map_buf.len() as u64)
                .expect("set length of todo file to output buffer");
            todo_file_handle
                .write_all(dir_map_buf.as_bytes())
                .expect("write update to todo file");
            todo_file_handle.flush().expect("flush todo file");
            todo_file_handle.sync_all().expect("sync todo file");
        }
        _ => {
            eprintln!("there are no todos to edit @: \"{pwd}\"");
        }
    };
    println!("updated todo: \"{}\" @ ID: {}", &update.new_text, update.id);
}

fn list_todos_pwd(dir_map_buf: &mut String, pwd: &str, todo_dir: &mut PathBuf) {
    // find for pwd
    let pwd_todo_path = dir_map_entries(&dir_map_buf).find(|(k, _v)| **k == *pwd);
    match pwd_todo_path {
        Some((pwd_path, todo_file_path_str)) => {
            use std::fmt::Write as _;
            let todo_file_path: &Path = todo_file_path_str.as_ref();
            // open the file
            let mut print_buf = String::with_capacity(
                todo_file_path
                    .metadata()
                    .map(|m| m.len() as usize + 256)
                    .unwrap_or(4096),
            );
            let todo_raw = with_pushed(todo_dir, todo_file_path, |path| {
                writeln!(&mut print_buf, "\nTodo: \"{}\"", pwd_path).unwrap();
                read_to_string(path).expect("load todo file")
            });
            write_todos_in_file(&mut print_buf, &todo_raw);
            println!("{print_buf}")
        }
        None => println!("No Todos @ PWD: \"{}\"", pwd),
    }
}

fn list_todos_all(dir_map_buf: &mut String, todo_dir: &mut PathBuf) {
    use std::fmt::Write as _;
    let dir_map_entries = dir_map_entries(&dir_map_buf);
    let mut print_buf = String::with_capacity(10_240);
    let mut in_buf = String::with_capacity(10_240);
    for (dir, file_name) in dir_map_entries {
        with_pushed(todo_dir, file_name, |path| {
            writeln!(&mut print_buf, "\nTodo: \"{}\"", &dir).unwrap();
            if let Some(mut handle) = OpenOptions::new()
                .read(true)
                .write(false)
                .create(false)
                .open(path)
                .inspect_err(|e| eprintln!("can't open {:?} due to {e}", &path))
                .ok()
            {
                handle
                    .read_to_string(&mut in_buf)
                    .inspect_err(|e| eprintln!("can't read {:?} due to {e}", &path))
                    .ok();
            }
        });
        write_todos_in_file(&mut print_buf, &in_buf);
        in_buf.clear();
    }
    println!("{print_buf}")
}

fn mark_done(done: Done, dir_map_buf: &mut String, pwd: &str, todo_dir: &mut PathBuf) {
    mark_status(MarkStatus::Done(done), dir_map_buf, pwd, todo_dir);
}

fn mark_active(active: Active, dir_map_buf: &mut String, pwd: &str, todo_dir: &mut PathBuf) {
    mark_status(MarkStatus::Active(active), dir_map_buf, pwd, todo_dir);
}

#[derive(Debug)]
enum MarkStatus {
    Done(Done),
    Active(Active),
}

impl From<&MarkStatus> for u64 {
    fn from(value: &MarkStatus) -> Self {
        match value {
            MarkStatus::Done(done) => done.index,
            MarkStatus::Active(active) => active.index,
        }
    }
}

fn mark_status(status: MarkStatus, dir_map_buf: &mut String, pwd: &str, todo_dir: &mut PathBuf) {
    use std::fmt::Write as _;
    #[derive(Debug)]
    enum Change<'line> {
        None(&'line str),
        Some((u64, &'line str)),
    }
    let id_want: u64 = (&status).into();
    let pwd_todo_path = dir_map_entries(&dir_map_buf).find(|(k, _v)| **k == *pwd);
    match pwd_todo_path {
        Some((pwd_path, todo_file_path_str)) => {
            let todo_file_path: &Path = todo_file_path_str.as_ref();
            // open the file
            let todo_raw = with_pushed(todo_dir, todo_file_path, |path| {
                read_to_string(path).expect("load todo file")
            });
            let mut new_todo_raw = String::with_capacity(todo_raw.len());

            let todo_records = todo_raw.lines().filter_map(|line| {
                let mut columns = line.split(COL_SEP_CH);

                columns.next().and_then(|id_str| {
                    id_str.parse::<u64>().ok().and_then(|id| {
                        if id == id_want {
                            let text = columns.next();
                            text.map(|t| Change::Some((id, t)))
                        } else {
                            Some(Change::None(line))
                        }
                    })
                })
            });
            for record in todo_records {
                match record {
                    Change::None(line) => writeln!(&mut new_todo_raw, "{line}").unwrap(),
                    Change::Some((id, text)) => match &status {
                        MarkStatus::Done(_done) => {
                            println!("Seting: \"{text}\" @: \"{pwd_path}\" to done...");
                            writeln!(&mut new_todo_raw, "{id}\t{text}\t{DONE_TODO}").unwrap()
                        }
                        MarkStatus::Active(_active) => {
                            println!("Seting: \"{text}\" @: \"{pwd_path}\" to active...");
                            writeln!(&mut new_todo_raw, "{id}\t{text}\t{ACTIVE_TODO}").unwrap()
                        }
                    },
                }
            }
            let mut todo_file_handle = with_pushed(todo_dir, todo_file_path_str, |path| {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .truncate(true)
                    .create(true)
                    .open(path)
                    .expect("open todo file")
            });
            todo_file_handle
                .write_all(new_todo_raw.as_bytes())
                .expect("write todo file");
        }
        None => println!("No Todos @ PWD: \"{}\"", pwd),
    }
}

fn delete_todo(
    delete_todo_id: DeleteTodoId,
    dir_map_buf: &mut String,
    pwd: &str,
    todo_dir: &mut PathBuf,
) {
    use std::fmt::Write as _;
    let id_to_delete = delete_todo_id.id;
    let pwd_todo_path = dir_map_entries(&dir_map_buf).find(|(k, _v)| **k == *pwd);
    match pwd_todo_path {
        Some((_pwd_path, todo_file_path_str)) => {
            let raw_old_todo = with_pushed(todo_dir, todo_file_path_str, |path| {
                read_to_string(path)
                    .expect("read todo file")
                    .into_boxed_str()
            });
            let mut lines: TinyVec<[(u64, &str); 20]> = TinyVec::new();
            let mut cancel = false;
            for line in raw_old_todo.lines() {
                if let Some((id, rest)) = line
                    .split_once(COL_SEP_CH)
                    .and_then(|(id_text, rest)| id_text.parse::<u64>().ok().map(|id| (id, rest)))
                {
                    if id != id_to_delete {
                        lines.push((id, rest));
                    } else {
                        let (text, status) = rest.split_once(COL_SEP_CH).unwrap_or_default();
                        if status == ACTIVE_TODO {
                            if prompt_delete_active() {
                                println!("deleting \"{text}\" at ID: {id}...");
                            } else {
                                println!("canceling...");
                                cancel = true;
                                break;
                            }
                        } else {
                            println!("deleting \"{text}\" at ID: {id}...");
                        }
                    }
                }
            }
            if !cancel {
                let renumbered_lines = lines.iter_mut().scan(0u64, |new_id, (_old_id, rest)| {
                    let out = (*new_id, rest);
                    *new_id += 1u64;
                    Some(out)
                });

                let mut out_buf = String::with_capacity(raw_old_todo.len());
                for line in renumbered_lines {
                    writeln!(&mut out_buf, "{}{COL_SEP_CH}{}", line.0, line.1).unwrap()
                }

                let mut todo_file_handle = with_pushed(todo_dir, todo_file_path_str, |path| {
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .truncate(true)
                        .create(false)
                        .open(path)
                        .expect("open todo file")
                });
                todo_file_handle
                    .write_all(out_buf.as_bytes())
                    .expect("write todo file");
            }
        }
        None => println!("No Todos @ PWD: \"{}\"", pwd),
    }
}

fn write_todos_in_file(print_buf: &mut String, raw_todo_file: &str) {
    use std::fmt::Write as _;
    let todo_records = raw_todo_file.lines().filter_map(|line| {
        let mut columns = line.split(COL_SEP_CH);
        let id = columns.next();
        let text = columns.next();
        let done = columns.next();

        done.and_then(|done| id.zip(text).map(|(id, text)| (id, text, done)))
    });
    for record in todo_records {
        let id = record.0;
        let text = record.1;
        let done = record.2;
        writeln!(print_buf, "{id} - {done}: {text}").unwrap()
    }
}

fn dir_map_entries(dir_map_buf: &str) -> impl Iterator<Item = (&str, &str)> {
    dir_map_buf.lines().map(|line| {
        let mut cols = line.split(COL_SEP_CH);
        (
            // key (dir)
            cols.next().expect("read key"),
            // value (number)
            cols.next().expect("read todo file name"),
        )
    })
}

/// Returns true if a newline or tab is found in text
fn reject_nl_and_tab(text: &str) -> bool {
    let tab = text.contains('\t');
    let nl = text.contains('\n');

    if tab {
        eprintln!("todo contains tab character");
    }

    if nl {
        eprintln!("todo contains newline character");
    }

    if nl || tab {
        eprintln!("can't create todo");
    }

    nl || tab
}
