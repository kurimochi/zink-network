#![no_main]
sp1_zkvm::entrypoint!(main);

use fibonacci_program::fibonacci;
use sp1_zkvm::io;

pub fn main() {
    let n: u32 = io::read();

    let fib = fibonacci(n);

    io::commit(&(fib, n));
}
