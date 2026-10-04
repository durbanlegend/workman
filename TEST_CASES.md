# Inline HTML test cases

---

## Image URL

### Raw

```html
<img src="https://raw.githubusercontent.com/lampsitter/egui_commonmark/master/assets/example-v4.png" alt="showcase" width=280/>
```

### Rendered

<img src="https://raw.githubusercontent.com/lampsitter/egui_commonmark/master/assets/example-v4.png" alt="showcase" width=280/>

---

## Details/summary structure

This will be permanently expanded and searchable if `html_prep::EXPAND_DETAILS` is `true`,
otherwise collapsible and not searchable.

### Raw

The inner code block has been indented here to prevented nested display issues. It would not be indented in practice.

```html
<details>
    <summary>Click <b>here</b> to view the error logs</summary>

    ```bash
    Error: Connection timeout at server.js:42
    ```
</details>
```

### Rendered

<details>
    <summary>Click <b>here</b> to view the error logs</summary>

```bash
Error: Connection timeout at server.js:42
```
</details>

---

## Embedded styling

### Raw

```md
* **Underline:** <u>This text will be underlined.</u>
* **Subscript:** H<sub>2</sub>O (Water)
* **Superscript:** The theorem is a<sup>2</sup> + b<sup>2</sup> = c<sup>2</sup>
* **Highlight:** You can use the <mark>mark tag</mark> to highlight text.
```

### Rendered

* **Underline:** <u>This text will be underlined.</u>
* **Subscript:** H<sub>2</sub>O (Water)
* **Superscript:** The theorem is a<sup>2</sup> + b<sup>2</sup> = c<sup>2</sup>
* **Highlight:** You can use the <mark>mark tag</mark> to highlight text.

---

## Keyboard tags

### Raw

```md
To save your current progress, press <kbd>Ctrl</kbd> + <kbd>S</kbd> on your keyboard.
```

### Rendered

To save your current progress, press <kbd>Ctrl</kbd> + <kbd>S</kbd> on your keyboard.

---

## Line breaks

The address lines each end in two spaces, causing a hard break

### Raw

```md
Company Name Registry  
123 Innovation Way  
<br>
Cape Town, South Africa  
7405
```

### Rendered

Company Name Registry  
123 Innovation Way  
<br>
Cape Town, South Africa  
7405

---

## Tables

### Raw

```html
<table>
  <tr>
    <th>Item</th>
    <th>Description</th>
    <th>Price</th>
  </tr>
  <tr>
    <td rowspan="2">Bundle A</td>
    <td>Includes the primary software suite and documentation tools.</td>
    <td>$49.00</td>
  </tr>
  <tr>
    <td>Bonus background pack.</td>
    <td>Free</td>
  </tr>
</table>
```

### Rendered

<table>
  <tr>
    <th>Item</th>
    <th>Description</th>
    <th>Price</th>
  </tr>
  <tr>
    <td rowspan="2">Bundle A</td>
    <td>Includes the primary software suite and documentation tools.</td>
    <td>$49.00</td>
  </tr>
  <tr>
    <td>Bonus background pack.</td>
    <td>Free</td>
  </tr>
</table>

---

## Links

### Raw

```html
Check out our product demo video below:

<iframe width="560" height="315" src="https://youtube.com" frameborder="0" allowfullscreen></iframe>
```

### Rendered

Check out our product demo video below:

<iframe width="560" height="315" src="https://youtube.com" frameborder="0" allowfullscreen></iframe>

---

## Legibility of un-annotated code blocks

### Raw

Does this code block render with enough contrast to be clearly legible?

    ```
    [package]
    name = "workman"

    ```

### Rendered

```
[package]
name = "workman"
```

---
