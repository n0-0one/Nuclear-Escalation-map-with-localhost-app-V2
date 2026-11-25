# How it works

## Reading the game

While War Thunder runs it serves a small read-only web page on your own computer (`http://127.0.0.1:8111`). The app asks it a few questions several times a second:

| Page | What the app takes from it |
|---|---|
| `/map_obj.json` | Every icon on your tactical map: position, colour, type |
| `/map_info.json` | Map size and grid |
| `/indicators` | Which vehicle you are in |
| `/state` | Altitude and speed |
| `/hudmsg` | The kill feed and the battle clock |
| `/mission.json` | Whether the battle is running |

Nothing else is read, and nothing is ever written to the game.

## Recognising the battle

- **Map** - runway positions are compared with every map in the data.
- **Side** - airfields painted in the friendly colour show which half of the map is yours.
- **Layout and BR bracket** - each BR bracket fields different air-defence systems and the two factions arrange theirs differently. Your own side's ground units are always on the map, so their pattern is a fingerprint that settles within seconds. A layout that expects friendly units where none has ever appeared loses points.
- **Tie-breakers** - unit names from the kill feed, and your own vehicle's battle rating (a battle cannot be in a bracket below it).

Every answer carries a confidence, and you can override any of them.

## Destroyed or just out of sight?

The mission only shows an enemy unit while a friendly unit or radar detects it, and for ten seconds after. So a unit leaving the map proves nothing by itself. The app calls a unit destroyed when, strongest first:

1. you marked it yourself;
2. the kill feed names that unit type **and** one such unit just left the map while its neighbours stayed visible, or only one exists - *confirmed*;
3. it left the map while neighbours in the same site stayed visible for longer than detection lingers - *probable*.

A kill the app cannot place yet is kept and matched later: as soon as every unit of that type but one has been seen alive since the kill, the remaining one is called *probable* (process of elimination). Until then the places it can still be are listed in the log and, when there are only a few, marked "?" on the map. A unit that shows up again is alive again, whatever was decided before. Units the mission restores on a timer show when they may be back.

Icons on the game's map are matched to mission units one to one, nearest first. The game sometimes draws every icon shifted by the same few dozen metres; the app measures that shift from all matched icons and takes it off before matching, so a kill is put on the right unit even then.

## Ships

Warships sail closed patrol routes from the mission. A ship on your map is followed and its real speed measured. When it drops off, it keeps moving along its route at that speed (dashed ring). A ship never seen is placed from the battle clock. A ship that vanishes while nearby units stay visible, or that the kill feed names, is marked sunk.

## Captured sites

Depots, strongpoints, assembly areas and airfields can be taken by AI columns. When icons of the other side sit on a site's unit positions for a few seconds, the site changes hands: its colour, the alerts and the strike targets follow the new owner. You can set the owner by hand in the site's details.

## Columns

The event's own rules, read from the game script, make columns predictable:

- they drive along the level's roads at a fixed speed and go straight for the last stretch;
- a fuel or ammo depot sends a column whenever its stock reaches a threshold, which works out to one every six minutes;
- supply columns go to one of the three nearest friendly storages; armoured columns pick one of the three nearest targets.

While a column is on your map the app follows it and works out which route it is on. When it drops off, the app keeps moving it along that road at the speed it was doing; the amber band shows the uncertainty, which grows the longer it stays unseen. Once one column from a depot has been seen, that depot's rhythm is known and its next departures are listed.

## Strike alerts

Every update the app counts live enemy units inside your chosen blast radius around each enemy site and each column. When a column is inside that radius of a depot or strongpoint, or will be within the warning time you set, the area glows softly on the map and on the mini map, and a quiet tone plays (optional). Friendly units inside the radius are counted and shown as a warning.

## What it cannot do

- Place aiming points for you (the game accepts them only from clicks on its own tactical map).
- See units that nobody on your team detects.
- Know the blast radius of each bomb (not in the game files) - you set it.
- Tell whether Gaijin will keep the local web page as it is.
