//! The levels, in the order of the pushes they need.  They were made for
//! this game (see `tests/levels.rs` for how) and are no other game's;
//! `pushes` is the fewest pushes that solve the level, which the solver
//! there confirms.

use crate::LevelDef;

pub const LEVELS: &[LevelDef] = &[
    LevelDef {
        pushes: 2,
        text: "
#######
#p.o.x#
#######",
    },
    LevelDef {
        pushes: 2,
        text: "
#######
#.x...#
#.o.o.#
#.p.x.#
#######",
    },
    LevelDef {
        pushes: 3,
        text: "
######
#p...#
#.o..#
#...x#
######",
    },
    LevelDef {
        pushes: 4,
        text: "
#######
#p..#x#
#.o...#
#.....#
#######",
    },
    LevelDef {
        pushes: 6,
        text: "
#######
#x.p..#
#.o...#
####..#
   ####",
    },
    LevelDef {
        pushes: 7,
        text: "
#########
#p.#....#
#..#....#
#xo.....#
#..#....#
#..###..#
#### ####",
    },
    LevelDef {
        pushes: 8,
        text: "
    #####
#####...#
#xpo....#
#.#.###.#
#...#...#
#...#...#
#########",
    },
    LevelDef {
        pushes: 9,
        text: "
######
#....##
#..oxx#
#..#op#
#..#..#
#######",
    },
    LevelDef {
        pushes: 10,
        text: "
######
#x...##
#xop..#
#.o#..#
#..#..#
#######",
    },
    LevelDef {
        pushes: 12,
        text: "
#######
#.xo..#
#..o..#
###...#
  #.P.#
  #####",
    },
    LevelDef {
        pushes: 13,
        text: "
#######
#p.#..#
#..#x.##
#xoo...#
#..#...#
########",
    },
    LevelDef {
        pushes: 13,
        text: "
      ####
#######..#
#Po......#
#o.......#
#.x..#...#
##########",
    },
    LevelDef {
        pushes: 14,
        text: "
#########
#...#...#
#...oox.#
#...#...#
#..x#...#
#...#p..#
#########",
    },
    LevelDef {
        pushes: 16,
        text: "
########
#......#
#x.##.o#
#poox.x#
#..#...#
########",
    },
    LevelDef {
        pushes: 17,
        text: "
##########
#..#.....#
#.xo.....#
#.o#.##..#
#xp#.....#
##########",
    },
    LevelDef {
        pushes: 17,
        text: "
########
#....ox#
#p..oox#
#....#x#
#..#...#
########",
    },
    LevelDef {
        pushes: 18,
        text: "
#########
#.xo....#
#...oox.#
#..##...#
#...#..##
#..x#.p#
########",
    },
    LevelDef {
        pushes: 19,
        text: "
########
#..##..#
#.o....#
#x##o..#
#Pxo...#
########",
    },
    LevelDef {
        pushes: 19,
        text: "
########
#......###
#p.......#
#.x#..xo.#
#..#..oo.#
#x.#.....#
##########",
    },
    LevelDef {
        pushes: 21,
        text: "
#####  ####
#...####..#
#.oxx.....#
####..#...#
#p..o.#####
#..#..#
#######",
    },
    LevelDef {
        pushes: 22,
        text: "
###########
#...#.....#
#...#..P.x#
#...#.....#
#.oo...##.#
#..xo.....#
###########",
    },
    LevelDef {
        pushes: 23,
        text: "
  #######
###.....#
#...ooox#
#....p#x#
#...#..x#
#...#...#
#########",
    },
    LevelDef {
        pushes: 24,
        text: "
#### ####
#..###..#
#....o..#
#..o#x.P#
#....ox.#
#########",
    },
    LevelDef {
        pushes: 26,
        text: "
##########
#....##..#
#..o..oxx#
#...#.#.P#
#...#.#o.#
#.....#..#
##########",
    },
    LevelDef {
        pushes: 27,
        text: "
  #########
###.x....P#
#..o...#..#
#..o#.....#
#.x..o....#
###########",
    },
    LevelDef {
        pushes: 28,
        text: "
###########
#.........#
#xx###.oo.#
#....#.x..#
#..p...oo.#
#.x.###...#
##### #####",
    },
    LevelDef {
        pushes: 28,
        text: "
##########
#...oP#..#
#..o.o#..#
#..##.#..#
##o.....x#
 #.x....x#
 #########",
    },
    LevelDef {
        pushes: 33,
        text: "
###########
#.x....#..#
#.x...o#.p#
#......#..#
#.###.oo..#
#x...x..o.#
###########",
    },
];
