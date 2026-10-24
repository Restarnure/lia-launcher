// Без цього релізна збірка на Windows відкриває ще й консоль, а її закриття вбиває лаунчер.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() { lia_lib::run() }
