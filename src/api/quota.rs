//! Local quota accounting (10,000 units/day, resets at midnight Pacific).
//! Each `*.list` call costs 1 unit; persisted in storage to survive restarts.
