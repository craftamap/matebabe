# matebabe

`matebabe` is a toy JVM built in Rust targeting Java 11. As it is just 
developed in order to learn, it has no certain goals besides running some small
programs that bring new challenges.

## Requirements

As `matebabe` does not come with it's own set of `.java` files for the required
JVM classes, `matebabe` reads classfiles from a local built of OpenJDK 11.

In order to build OpenJDK, [clone](https://github.com/openjdk/jdk11u-dev/),
and run:

```sh
bash configure --disable-warnings-as-errors
make
```
Note: building OpenJDK may already require a Java Installation.

This will built the whole JDK, although we are just interested in building 
the classes. After building is done, you may need to adjust the `class_path`
variable in `src/run.rs`, depending on where the final build files are located 
relative to the directory `matebabe` is executed in.

`matebabe` itself is build in Rust, so a recent version of Rust is also 
required.

## Usage

In order to run a class file, use `matebabe run <classname>`, eg: `matebabe run
Main`
