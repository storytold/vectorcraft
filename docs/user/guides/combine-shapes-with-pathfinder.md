---
title: Combine shapes with Pathfinder
description: Unite, cut, intersect and split overlapping shapes with the Pathfinder panel.
order: 30
verified: 0.10.0
---

Select two or more overlapping shapes, open **Window** › **Pathfinder**, and click the operation you want.

## Combine shapes

1. Select the shapes with the **Selection Tool** (shortcut V). Hold Shift and click to add shapes.
2. Choose **Window** › **Pathfinder** to open the panel.
3. Under **Shape Modes:** or **Pathfinders:**, click one operation. Hover over a button to see its name.

The buttons are disabled until enough shapes are selected. Every operation needs two shapes, except **Divide** and **Outline**, which work on one.

## What each operation does

The result depends on which shape is in front.

| Button      | Result                                                                             |
|-------------|------------------------------------------------------------------------------------|
| Unite       | One shape from the union of everything. It keeps the front-most shape's look.      |
| Minus Front | The back-most shape minus everything in front of it.                               |
| Intersect   | Only the area covered by all the shapes.                                           |
| Exclude     | The area covered by an odd number of shapes.                                       |
| Divide      | Splits the shapes into every non-overlapping piece. An open path splits the filled shapes it crosses. |
| Trim        | Removes the hidden parts. Each shape that stays visible remains its own object.    |
| Merge       | Trims, then merges touching or overlapping visible parts that share the same look. |
| Crop        | Keeps the parts of the lower shapes that lie inside the front-most shape.          |
| Outline     | Splits every edge where shapes cross and returns open paths.                       |
| Minus Back  | The front-most shape minus everything behind it.                                   |

## Repeat the last operation

Click the **Panel menu** button of the Pathfinder panel and choose the **Repeat** entry. It is named after the last operation, for example **Repeat Unite**. It is disabled until you have run an operation, and while nothing is selected.

## Other ways to cut shapes apart

The **Shape Builder Tool** (shortcut Shift+M) works on selected paths and compound paths. It merges, or deletes, the regions that the paths make where they overlap.
