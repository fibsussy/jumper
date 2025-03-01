pub mod tmux;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use dirs::home_dir;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{metadata, remove_file, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
extern crate whoami;

#[derive(Debug, Parser)]
#[command(name = "Jumper", about = "fzf through a list of projects")]
struct Opt {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Add a project to the .projects file
    #[command(name = "add", aliases = &["a"])]
    Add {
        /// The project directory to add. If not provided, the current directory will be added.
        dir: Option<String>,
    },
    /// Delete a project from the .projects file
    #[command(name = "delete", aliases = &["del", "d"])]
    Delete,
    /// List all projects in the .projects file
    #[command(name = "list", aliases = &["ls", "l"])]
    List,
    /// Display the contents of the .projects file
    #[command(name = "status", aliases = &["stat", "s"])]
    Status,
    /// Set or remove depth for a project
    #[command(name = "set-depth", aliases = &["depth", "sd"])]
    SetDepth,
    /// Clear the cache file
    #[command(name = "clear-cache", aliases = &["cc"])]
    ClearCache,
    /// Generate shell completion scripts
    #[command(name = "completion", aliases = &["comp", "c"])]
    Completion {
        /// The shell to generate the script for (e.g., bash, zsh, fish, powershell, elvish)
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Debug, Clone)]
struct Project {
    path: String,
}

impl Project {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }

    fn to_fzf_display(&self) -> String {
        let user = whoami::username();
        self.path
            .replace(&format!("/home/{user}"), "~")
            .replace("/run/media/fib/ExternalSSD/code", "code")
            .replace('.', "")
    }
}

fn main() {
    let opt = Opt::parse();
    match opt.command {
        Some(Commands::Add { dir }) => add_project(dir.as_deref()),
        Some(Commands::Delete) => delete_project(),
        Some(Commands::List) => list_projects(),
        Some(Commands::Status) => status_projects(),
        Some(Commands::SetDepth) => set_depth(),
        Some(Commands::ClearCache) => clear_cache(),
        Some(Commands::Completion { shell }) => generate_completion(shell),
        None => main_execution(),
    }
}

fn generate_completion(shell: Shell) {
    let mut cmd = Opt::command();
    let bin_name = env!("CARGO_PKG_NAME");
    generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
}

fn get_home_path(file: &str) -> PathBuf {
    home_dir()
        .expect("Unable to find home directory")
        .join(file)
}

fn touch_file(path: &PathBuf) {
    OpenOptions::new()
        .create(true)
        .write(true)
        .open(path)
        .unwrap();
}

fn read_lines<P>(filename: P) -> std::io::Result<Vec<String>>
where
    P: AsRef<std::path::Path>,
{
    let file = File::open(filename)?;
    let buf = BufReader::new(file);
    buf.lines().collect()
}

fn write_lines<P>(filename: P, lines: &[String]) -> std::io::Result<()>
where
    P: AsRef<std::path::Path>,
{
    let mut file = File::create(filename)?;
    for line in lines {
        writeln!(file, "{line}")?;
    }
    Ok(())
}

fn add_project(dir: Option<&str>) {
    let projects_file = get_home_path(".projects");
    touch_file(&projects_file);
    let current_dir = env::current_dir().unwrap().to_str().unwrap().to_string();
    let dir = dir.unwrap_or(&current_dir).to_string();
    let mut lines = read_lines(&projects_file).unwrap_or_else(|_| vec![]);
    if !lines.contains(&dir) {
        lines.push(dir.clone());
    }
    write_lines(&projects_file, &lines).unwrap();
    println!("Added \"{dir}\" to .projects");
}

fn delete_project() {
    let projects_file = get_home_path(".projects");
    let lines = read_lines(&projects_file).unwrap_or_else(|_| vec![]);
    let mut selected = Command::new("fzf")
        .arg("--reverse")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to execute fzf");
    {
        let fzf_stdin = selected.stdin.as_mut().expect("Failed to open fzf stdin");
        fzf_stdin
            .write_all(lines.join("\n").as_bytes())
            .expect("Failed to write to fzf stdin");
    }
    let output = selected
        .wait_with_output()
        .expect("Failed to read fzf output");
    if !output.stdout.is_empty() {
        let selected_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let new_lines: Vec<String> = lines
            .into_iter()
            .filter(|line| line != &selected_str)
            .collect();
        write_lines(&projects_file, &new_lines).unwrap();
        println!("Deleted \"{selected_str}\" from .projects");
    }
}

fn get_tmux_sessions() -> Vec<Project> {
    let mut projects = Vec::new();
    let tmux_list_output = Command::new("tmux")
        .arg("list-sessions")
        .arg("-F")
        .arg("#{session_name}")
        .output()
        .expect("Failed to list tmux sessions");
    dbg!(&tmux_list_output);
    if tmux_list_output.status.success() {
        let tmux_sessions = String::from_utf8_lossy(&tmux_list_output.stdout);
        for session in tmux_sessions.lines() {
            if let Some(session_name) = session.split(':').next() {
                projects.push(Project::new(session_name));
            }
        }
    }
    projects
}

fn get_projects() -> Vec<Project> {
    let projects_file = get_home_path(".projects");
    let mut projects = Vec::new();
    let mut unique_projects = HashSet::new();

    if let Ok(lines) = read_lines(&projects_file) {
        let re = Regex::new(r"(.*) --depth (\d+)").unwrap();
        for line in lines {
            if let Some(captures) = re.captures(&line) {
                let dir = captures.get(1).unwrap().as_str();
                let depth = captures.get(2).unwrap().as_str().parse::<u32>().unwrap();
                projects.push(Project::new(dir));
                let sub_dirs = Command::new("find")
                    .arg("-L")
                    .arg(dir)
                    .arg("-maxdepth")
                    .arg(depth.to_string())
                    .arg("-type")
                    .arg("d")
                    .output()
                    .expect("Failed to execute find");
                let sub_dirs = String::from_utf8_lossy(&sub_dirs.stdout);
                for sub_dir in sub_dirs.lines() {
                    projects.push(Project::new(sub_dir));
                }
            } else {
                projects.push(Project::new(&line));
            }
        }
    }

    projects.extend(get_tmux_sessions());

    projects
        .into_iter()
        .filter(|project| unique_projects.insert(project.path.clone()))
        .collect()
}

fn reorder_projects_by_history(history: &[String], projects: &[Project]) -> Vec<Project> {
    let mut reordered_projects = Vec::new();
    let mut seen = HashSet::new();
    let projects_map: HashMap<String, &Project> =
        projects.iter().map(|p| (p.to_fzf_display(), p)).collect();
    for hist in history {
        if let Some(project) = projects_map.get(hist) {
            if seen.insert(project.path.clone()) {
                reordered_projects.push((*project).clone());
            }
        }
    }
    for project in projects {
        if seen.insert(project.path.clone()) {
            reordered_projects.push(project.clone());
        }
    }
    reordered_projects
}

fn move_to_tmux_session(dir: &Project) {
    let tmux_session_name_og = dir.to_fzf_display();
    let tmux_session_name = tmux_session_name_og.replace('~', "\\~");

    // Check if the session already exists
    let tmux_session_already_exists = tmux::session_exists(&tmux_session_name_og);

    // Create a new tmux session if it doesn't exist
    if !tmux_session_already_exists && !tmux::create_session(&tmux_session_name_og, &dir.path) {
        eprintln!("Failed to create new tmux session");
        return;
    }

    // Determine if running inside a tmux session
    let is_inside_tmux = env::var("TMUX").is_ok();

    if is_inside_tmux {
        // Running inside tmux: switch client to the session
        if !tmux::switch_client(&tmux_session_name) {
            eprintln!("Failed to switch tmux client");
        }
    } else {
        // Running outside tmux: attach to the session
        if !tmux::attach_session(&tmux_session_name) {
            eprintln!("Failed to attach to tmux session");
        }
    }
}

fn get_current_session() -> Option<String> {
    let output = Command::new("tmux")
        .arg("display-message")
        .arg("-p")
        .arg("#S")
        .output()
        .expect("Failed to execute tmux command");
    if output.status.success() {
        let session_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(session_name)
    } else {
        eprintln!("Failed to get current tmux session name");
        None
    }
}

fn main_execution() {
    let projects_history_file = get_home_path(".projects_history");
    touch_file(&projects_history_file);
    let history_lines = read_lines(&projects_history_file).unwrap_or_else(|_| vec![]);
    let projects_file = get_home_path(".projects");
    let cache_file = PathBuf::from("/tmp/.projects_cache");
    let projects_metadata =
        metadata(&projects_file).expect("Unable to read projects file metadata");
    let cache_metadata = metadata(&cache_file).ok();
    let projects_last_modified = projects_metadata
        .modified()
        .expect("Unable to get modified time");
    let projects = if cache_metadata.is_some()
        && cache_metadata
            .unwrap()
            .modified()
            .expect("Unable to get cache modified time")
            >= projects_last_modified
    {
        read_lines(&cache_file)
            .unwrap()
            .into_iter()
            .map(|line| Project::new(&line))
            .collect()
    } else {
        let new_projects = get_projects();
        let project_paths: Vec<String> = new_projects.iter().map(|p| p.path.clone()).collect();
        write_lines(&cache_file, &project_paths).unwrap();
        new_projects
    };
    let reordered_projects = reorder_projects_by_history(&history_lines, &projects);
    let current_session = get_current_session();
    let project_set: HashSet<_> = projects
        .iter()
        .filter_map(|p| {
            if let Some(current_session) = &current_session {
                if Project::new(&p.path).to_fzf_display() == *current_session {
                    return None;
                }
            }
            Some(p.to_fzf_display())
        })
        .collect();
    let mut fzf_through: Vec<String> =
        Vec::with_capacity(history_lines.len() + reordered_projects.len());
    let mut seen = HashSet::new();
    for item in &history_lines {
        if project_set.contains(item) && seen.insert(item.clone()) {
            fzf_through.push(item.clone());
        }
    }
    for project in &reordered_projects {
        if seen.insert(project.to_fzf_display()) {
            fzf_through.push(project.to_fzf_display());
        }
    }
    let mut selected = Command::new("fzf")
        .arg("--reverse")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to execute fzf");
    {
        let fzf_stdin = selected.stdin.as_mut().expect("Failed to open fzf stdin");
        fzf_stdin
            .write_all(fzf_through.join("\n").as_bytes())
            .expect("Failed to write to fzf stdin");
    }
    let output = selected
        .wait_with_output()
        .expect("Failed to read fzf output");
    let selected_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if selected_str.is_empty() {
        return;
    }
    let mut new_history = vec![selected_str.clone()];
    new_history.extend(
        history_lines
            .iter()
            .filter(|&item| item != &selected_str)
            .cloned(),
    );
    new_history.truncate(2000);
    write_lines(&projects_history_file, &new_history).unwrap();
    if let Some(idx) = reordered_projects
        .iter()
        .position(|p| p.to_fzf_display() == selected_str)
    {
        let dir = reordered_projects.get(idx).unwrap();
        move_to_tmux_session(dir);
    } else {
        println!("L");
    }
}

fn list_projects() {
    let projects = get_projects();
    for project in projects {
        println!("{}", project.path);
    }
}

fn status_projects() {
    let projects_file = get_home_path(".projects");
    let lines = read_lines(&projects_file).unwrap_or_else(|_| vec![]);
    for line in lines {
        println!("{line}");
    }
}

fn set_depth() {
    let projects_file = get_home_path(".projects");
    let lines = read_lines(&projects_file).unwrap_or_else(|_| vec![]);
    let mut selected = Command::new("fzf")
        .arg("--reverse")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to execute fzf");
    {
        let fzf_stdin = selected.stdin.as_mut().expect("Failed to open fzf stdin");
        fzf_stdin
            .write_all(lines.join("\n").as_bytes())
            .expect("Failed to write to fzf stdin");
    }
    let output = selected
        .wait_with_output()
        .expect("Failed to read fzf output");
    if !output.stdout.is_empty() {
        let selected_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        println!(
            "Set depth for {selected_str}: (Press Enter to remove depth, Ctrl+C to cancel)"
        );
        let mut depth_input = String::new();
        std::io::stdin()
            .read_line(&mut depth_input)
            .expect("Failed to read depth input");
        let depth_input = depth_input.trim();
        let re = Regex::new(r"(.*) --depth \d+").unwrap();
        let mut new_lines: Vec<String> = lines
            .into_iter()
            .filter(|line| !re.is_match(line) || !line.starts_with(&selected_str))
            .collect();
        if depth_input.is_empty() {
            new_lines.push(selected_str.clone());
        } else {
            new_lines.push(format!("{selected_str} --depth {depth_input}"));
        }
        new_lines.sort();
        write_lines(&projects_file, &new_lines).unwrap();
        println!("Set depth for \"{selected_str}\" to {depth_input}");
    }
}

fn clear_cache() {
    let cache_file = PathBuf::from("/tmp/.projects_cache");
    if cache_file.exists() {
        remove_file(&cache_file).expect("Failed to delete cache file");
        println!("Cache cleared");
    } else {
        println!("No cache file found");
    }
}
