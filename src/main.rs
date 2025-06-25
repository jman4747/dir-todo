/// dir map is "path" => "hash of path"
use std::{
    env::{current_dir, home_dir},
    ffi::OsStr,
    fmt::Display,
    fs::{OpenOptions, create_dir, read_to_string, rename},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use argh::FromArgs;

const TODO_DIR_NAME: &str = "todo";
const DIR_MAP_NAME: &str = "dirmap.tsv";
const DIR_MAP_NEW_NAME: &str = "dirmap.new.tsv";
const LINE_SEP_CH: char = '\n';
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
    match todo.cmd {
        Command::New(text) => {
            let pwd_todo_map_entry = dir_map_entries.iter().find(|(k, _v)| **k == *pwd);
            let new_idx = match pwd_todo_map_entry {
                Some((_path, index)) => {
                    // dir_map_entries.push((Some(pwd), Some(path_hash)));
                    // create file
                    let (exists, mut todo_file_handle) =
                        with_pushed(&mut todo_dir, index, |path| {
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

                    let next_idx = if exists {
                        todo_buf
                            .lines()
                            .map(|line| {
                                line.split_once(COL_SEP_CH)
                                    .map(|(idx, _rest)| idx.parse::<u64>().unwrap_or(0))
                                    .unwrap_or(0)
                            })
                            .max()
                            .map(|max| max + 1)
                            .unwrap_or(0)
                    } else {
                        0
                    };

                    text.io_write_as_active(&mut todo_file_handle, next_idx)
                        .expect("write new todo to file");

                    next_idx
                }

                _ => {
                    let mut out_buf = String::with_capacity(20 + 4);
                    use std::fmt::Write as _;
                    let path_hash = calculate_hash(&pwd);
                    write!(&mut out_buf, "{path_hash}.tsv").unwrap();
                    // create file
                    let mut todo_file_handle = with_pushed(&mut todo_dir, &out_buf, |path| {
                        OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .append(true)
                            .open(path)
                    })
                    .expect("open todo");

                    text.io_write_as_active(&mut todo_file_handle, 0)
                        .expect("write new todo to file");

                    write!(&mut dir_map_buf, "{pwd}\t{}\n", out_buf).unwrap();
                    save_dir_map(&mut todo_dir, &mut dir_map_buf)
                        .expect("write new entry to dir map");
                    0
                }
            };
            println!("added todo: \"{}\" at ID: {new_idx}", &text.text);
        }
        Command::List(all) if all.all => {}
        Command::List(_all) => {
            // find for pwd
            let pwd_todo_path = dir_map_entries.iter().find(|(k, _v)| **k == *pwd);
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
                    let todo_raw = with_pushed(&mut todo_dir, todo_file_path, |path| {
                        writeln!(&mut print_buf, "\nTodo: \"{}\"", pwd_path).unwrap();
                        read_to_string(path).expect("load todo file")
                    });
                    let todo_records = todo_raw.lines().filter_map(|line| {
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
                        writeln!(&mut print_buf, "{id} - {done}: {text}").unwrap()
                    }
                    println!("{print_buf}")
                }
                None => println!("No Todos @ PWD: \"{}\"", pwd),
            }
        }
        Command::Update(update_todo) => todo!(),
        Command::Delete(delete_todo) => todo!(),
        Command::Done(done_id) => {
            use std::fmt::Write as _;
            #[derive(Debug)]
            enum Change<'line> {
                None(&'line str),
                Some((u64, &'line str)),
            }
            let id_want = done_id.index;
            let pwd_todo_path = dir_map_entries.iter().find(|(k, _v)| **k == *pwd);
            match pwd_todo_path {
                Some((pwd_path, todo_file_path_str)) => {
                    let todo_file_path: &Path = todo_file_path_str.as_ref();
                    // open the file
                    let todo_raw = with_pushed(&mut todo_dir, todo_file_path, |path| {
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
                            Change::Some((id, text)) => {
                                println!("Seting: \"{text}\" @: \"{pwd_path}\" to done...");
                                writeln!(&mut new_todo_raw, "{id}\t{text}\t{DONE_TODO}").unwrap()
                            }
                        }
                    }
                    let mut todo_file_handle =
                        with_pushed(&mut todo_dir, todo_file_path_str, |path| {
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
    }
}

#[derive(Debug, FromArgs, PartialEq)]
/// Directory mapped TODO
struct Todo {
    #[argh(subcommand)]
    cmd: Command,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
pub enum Command {
    New(NewTodo),
    List(ListTodo),
    Update(UpdateTodo),
    Delete(DeleteTodo),
    Done(Done), // active
}

#[derive(FromArgs, PartialEq, Debug)]
/// Create a todo.
#[argh(subcommand, name = "new")]
struct NewTodo {
    #[argh(positional)]
    text: String,
}

impl NewTodo {
    pub fn active_record_buff_size(&self) -> usize {
        self.text.len() + 2 + 1 + ACTIVE_TODO.len() + 2
    }

    pub fn io_write_as_active(
        &self,
        buf: &mut impl std::io::Write,
        id: u64,
    ) -> std::io::Result<()> {
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
/// List todos.
#[argh(subcommand, name = "list")]
struct ListTodo {
    #[argh(switch, short = 'a')]
    /// list all Todos regardless of directory
    all: bool,
}

#[derive(FromArgs, PartialEq, Debug)]
/// Update a todo.
#[argh(subcommand, name = "update")]
struct UpdateTodo {
    #[argh(positional)]
    /// path index todo
    path_index: PathBuf,
    #[argh(positional)]
    /// todo number
    number: usize,
}

#[derive(FromArgs, PartialEq, Debug)]
/// Delete a todo.
#[argh(subcommand, name = "delete")]
struct DeleteTodo {
    #[argh(positional)]
    /// path to todo
    path: PathBuf,
    #[argh(positional)]
    /// todo number
    number: usize,
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

fn with_pushed_and_ext<P, F, Out, E>(buf: &mut PathBuf, to_push: P, ext: E, mut f: F) -> Out
where
    P: AsRef<Path>,
    F: FnMut(&Path) -> Out,
    E: AsRef<OsStr>,
{
    buf.push(to_push);
    buf.set_extension(ext);
    let out = f(buf.as_path());
    buf.pop();
    out
}

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
