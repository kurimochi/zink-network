#![no_main]
sp1_zkvm::entrypoint!(main);

use fibonacci_program::fibonacci;
use sp1_zkvm::io;

pub fn main() {
    let n: u32 = io::read(); // 入力を読み込む

    let fib = fibonacci(n); // フィボナッチ数列の第n項を計算

    io::commit(&(fib, n)); // commitの引数に関して証明が行われる
}
